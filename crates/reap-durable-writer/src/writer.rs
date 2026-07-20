use std::error::Error;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error as ThisError;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::bounded::{BoundedSink, PendingRecord};
use crate::lease::{DurableLease, LeaseError, normalize_journal_path};
use crate::progress::WriterProgress;

pub trait JournalCodec<Record>: Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    fn encode(&self, record: &Record, output: &mut Vec<u8>) -> Result<(), Self::Error>;

    fn durability_error(&self, error: &WriterError<Self::Error>) -> String {
        error.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct DurableWriterConfig {
    pub path: PathBuf,
    pub channel_capacity: usize,
    pub flush_every_records: usize,
}

#[derive(Debug, ThisError)]
pub enum WriterError<CodecError>
where
    CodecError: Error + 'static,
{
    #[error(transparent)]
    Lease(#[from] LeaseError),
    #[error("durable writer IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("durable writer codec failed: {0}")]
    Codec(#[source] CodecError),
    #[error("durable lease for {lease_path} does not match configured journal {config_path}")]
    LeaseMismatch {
        config_path: PathBuf,
        lease_path: PathBuf,
    },
    #[error("durable writer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub struct DurableWriterRuntime<Record, Codec>
where
    Codec: JournalCodec<Record>,
{
    sink: BoundedSink<Record>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), WriterError<Codec::Error>>>>,
    _lease: Arc<DurableLease>,
    _codec: PhantomData<fn() -> Codec>,
}

impl<Record, Codec> DurableWriterRuntime<Record, Codec>
where
    Codec: JournalCodec<Record>,
{
    #[must_use]
    pub fn sink(&self) -> BoundedSink<Record> {
        self.sink.clone()
    }

    pub fn request_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    pub async fn stop_writer(&mut self) -> Result<(), WriterError<Codec::Error>> {
        self.request_shutdown();
        let result = match self.task.as_mut() {
            Some(task) => Some(task.await),
            None => None,
        };
        self.task.take();
        if let Some(result) = result {
            result??;
        }
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<(), WriterError<Codec::Error>> {
        self.stop_writer().await
    }
}

impl<Record, Codec> Drop for DurableWriterRuntime<Record, Codec>
where
    Codec: JournalCodec<Record>,
{
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub async fn start_durable_writer<Record, Codec>(
    config: DurableWriterConfig,
    codec: Codec,
) -> Result<DurableWriterRuntime<Record, Codec>, WriterError<Codec::Error>>
where
    Record: Send + Sync + 'static,
    Codec: JournalCodec<Record>,
{
    let lease = DurableLease::acquire(&config.path)?;
    start_durable_writer_with_lease(config, lease, codec).await
}

pub async fn start_durable_writer_with_lease<Record, Codec>(
    mut config: DurableWriterConfig,
    lease: DurableLease,
    codec: Codec,
) -> Result<DurableWriterRuntime<Record, Codec>, WriterError<Codec::Error>>
where
    Record: Send + Sync + 'static,
    Codec: JournalCodec<Record>,
{
    let normalized_config_path = normalize_journal_path(&config.path)?;
    if normalized_config_path != lease.journal_path() {
        return Err(WriterError::LeaseMismatch {
            config_path: normalized_config_path,
            lease_path: lease.journal_path().to_path_buf(),
        });
    }
    config.path = lease.journal_path().to_path_buf();
    let file = open_writer_file(&config).await?;
    let lease = Arc::new(lease);
    let channel_capacity = config.channel_capacity.max(1);
    let (sender, receiver) = mpsc::channel(channel_capacity);
    let (shutdown, shutdown_rx) = oneshot::channel();
    let progress = Arc::new(WriterProgress::new(channel_capacity));
    let sink = BoundedSink {
        sender,
        progress: Arc::clone(&progress),
    };
    let writer_lease = Arc::clone(&lease);
    let task = tokio::spawn(async move {
        let _lease = writer_lease;
        run_writer(config, file, receiver, shutdown_rx, progress, codec).await
    });
    Ok(DurableWriterRuntime {
        sink,
        shutdown: Some(shutdown),
        task: Some(task),
        _lease: lease,
        _codec: PhantomData,
    })
}

async fn open_writer_file<CodecError>(
    config: &DurableWriterConfig,
) -> Result<tokio::fs::File, WriterError<CodecError>>
where
    CodecError: Error + 'static,
{
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.path)
        .await?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&config.path, private_permissions()).await?;
    Ok(file)
}

async fn run_writer<Record, Codec>(
    config: DurableWriterConfig,
    mut file: tokio::fs::File,
    mut receiver: mpsc::Receiver<PendingRecord<Record>>,
    mut shutdown: oneshot::Receiver<()>,
    progress: Arc<WriterProgress>,
    codec: Codec,
) -> Result<(), WriterError<Codec::Error>>
where
    Record: Sync,
    Codec: JournalCodec<Record>,
{
    let flush_every = config.flush_every_records.max(1);
    let mut since_flush = 0_usize;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                receiver.close();
                while let Some(mut pending) = receiver.recv().await {
                    pending.mark_received();
                    let result = write_pending(&mut file, pending, &progress, &codec).await;
                    progress.record_completed();
                    result?;
                }
                break;
            }
            pending = receiver.recv() => {
                let Some(mut pending) = pending else { break; };
                pending.mark_received();
                let result = write_pending(&mut file, pending, &progress, &codec).await;
                progress.record_completed();
                let durable = result?;
                if durable {
                    since_flush = 0;
                } else {
                    since_flush += 1;
                }
                if !durable && since_flush >= flush_every {
                    if let Err(error) = file.flush().await {
                        progress.record_sync_failure();
                        return Err(error.into());
                    }
                    progress.record_writer_progress();
                    since_flush = 0;
                }
            }
        }
    }
    if let Err(error) = file.flush().await {
        progress.record_sync_failure();
        return Err(error.into());
    }
    progress.record_writer_progress();
    if let Err(error) = file.sync_data().await {
        progress.record_sync_failure();
        return Err(error.into());
    }
    progress.record_writer_progress();
    Ok(())
}

async fn write_pending<Record, Codec>(
    file: &mut tokio::fs::File,
    pending: PendingRecord<Record>,
    progress: &WriterProgress,
    codec: &Codec,
) -> Result<bool, WriterError<Codec::Error>>
where
    Record: Sync,
    Codec: JournalCodec<Record>,
{
    let PendingRecord {
        record,
        durable_ack,
        ..
    } = pending;
    let durable = durable_ack.is_some();
    let result = async {
        let mut line = Vec::new();
        if let Err(error) = codec.encode(&record, &mut line) {
            progress.record_write_failure();
            return Err(WriterError::Codec(error));
        }
        line.push(b'\n');
        if let Err(error) = file.write_all(&line).await {
            progress.record_write_failure();
            return Err(WriterError::Io(error));
        }
        progress.record_written();
        if durable {
            if let Err(error) = file.flush().await {
                progress.record_sync_failure();
                return Err(WriterError::Io(error));
            }
            progress.record_writer_progress();
            if let Err(error) = file.sync_data().await {
                progress.record_sync_failure();
                return Err(WriterError::Io(error));
            }
            progress.record_durable_sync_completion();
        }
        Ok::<(), WriterError<Codec::Error>>(())
    }
    .await;
    if let Some(acknowledgement) = durable_ack {
        let acknowledgement_result = match &result {
            Ok(()) => Ok(()),
            Err(error) => Err(codec.durability_error(error)),
        };
        let _ = acknowledgement.send(acknowledgement_result);
    }
    result.map(|()| durable)
}

#[cfg(unix)]
fn private_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o600)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    struct BytesCodec;

    impl JournalCodec<u8> for BytesCodec {
        type Error = Infallible;

        fn encode(&self, _record: &u8, output: &mut Vec<u8>) -> Result<(), Self::Error> {
            output.extend_from_slice(b"{}");
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn write_and_sync_failures_are_counted_without_false_durability() {
        let directory = tempfile::tempdir().unwrap();
        let read_only = std::fs::File::open(directory.path()).unwrap();
        let mut read_only = tokio::fs::File::from_std(read_only);
        let progress = WriterProgress::new(1);
        assert!(
            !write_pending(
                &mut read_only,
                PendingRecord {
                    record: 1_u8,
                    durable_ack: None,
                    queue_drop_guard: None,
                },
                &progress,
                &BytesCodec,
            )
            .await
            .unwrap()
        );
        let error = write_pending(
            &mut read_only,
            PendingRecord {
                record: 2_u8,
                durable_ack: None,
                queue_drop_guard: None,
            },
            &progress,
            &BytesCodec,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, WriterError::Io(_)));
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.records_written, 1);
        assert_eq!(snapshot.write_failures, 1);

        let write_only = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut no_sync = tokio::fs::File::from_std(write_only);
        let progress = WriterProgress::new(1);
        let (durable_ack, acknowledgement) = oneshot::channel();
        let error = write_pending(
            &mut no_sync,
            PendingRecord {
                record: 2_u8,
                durable_ack: Some(durable_ack),
                queue_drop_guard: None,
            },
            &progress,
            &BytesCodec,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, WriterError::Io(_)));
        assert!(acknowledgement.await.unwrap().is_err());
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.records_written, 1);
        assert_eq!(snapshot.durable_sync_completions, 0);
        assert_eq!(snapshot.sync_failures, 1);
    }
}

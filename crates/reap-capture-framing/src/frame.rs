use std::io::Write;

use serde::Serialize;
use thiserror::Error;

/// Serializes one typed value as compact JSON followed by exactly one newline.
///
/// This compatibility encoder allocates before enforcing any frame ceiling.
/// New call sites must use [`encode_jsonl_frame_bounded`].
#[cfg(feature = "legacy-reap-capture")]
pub fn encode_jsonl_frame_legacy_unbounded<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    T: Serialize,
{
    let mut frame = serde_json::to_vec(value)?;
    frame.push(b'\n');
    Ok(frame)
}

#[derive(Debug, Error)]
pub enum BoundedJsonlFrameError {
    #[error("JSONL serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error(
        "JSONL frame exceeds {limit_bytes} bytes (observed at least {observed_at_least_bytes})"
    )]
    FrameTooLarge {
        observed_at_least_bytes: usize,
        limit_bytes: usize,
    },
    #[error(
        "JSONL serialization size changed between bounded passes: measured {measured_bytes}, second pass observed {second_pass_bytes} bytes"
    )]
    SizeChanged {
        measured_bytes: usize,
        second_pass_bytes: usize,
    },
}

/// Encodes compact JSONL without allocating a frame buffer before its exact
/// size is proven to fit `max_frame_bytes`.
///
/// The first serialization pass writes into a zero-allocation counter. The
/// second pass writes into a fixed-capacity buffer and fails closed if a
/// stateful serializer produces a different size.
pub fn encode_jsonl_frame_bounded<T>(
    value: &T,
    max_frame_bytes: usize,
) -> Result<Vec<u8>, BoundedJsonlFrameError>
where
    T: Serialize,
{
    let mut counter = CountingWriter::new(max_frame_bytes.saturating_sub(1));
    if let Err(source) = serde_json::to_writer(&mut counter, value) {
        if counter.overflowed {
            return Err(BoundedJsonlFrameError::FrameTooLarge {
                observed_at_least_bytes: max_frame_bytes.saturating_add(1),
                limit_bytes: max_frame_bytes,
            });
        }
        return Err(BoundedJsonlFrameError::Serialization(source));
    }
    let measured_bytes = counter.bytes.saturating_add(1);
    if measured_bytes > max_frame_bytes {
        return Err(BoundedJsonlFrameError::FrameTooLarge {
            observed_at_least_bytes: measured_bytes,
            limit_bytes: max_frame_bytes,
        });
    }

    let mut bounded = FixedCapacityWriter::new(measured_bytes);
    let serialization = serde_json::to_writer(&mut bounded, value);
    if let Err(source) = serialization {
        if bounded.overflowed {
            return Err(BoundedJsonlFrameError::SizeChanged {
                measured_bytes,
                second_pass_bytes: bounded.observed_bytes.saturating_add(1),
            });
        }
        return Err(BoundedJsonlFrameError::Serialization(source));
    }
    bounded.write_newline();
    if bounded.overflowed || bounded.bytes.len() != measured_bytes {
        return Err(BoundedJsonlFrameError::SizeChanged {
            measured_bytes,
            second_pass_bytes: bounded.observed_bytes,
        });
    }
    Ok(bounded.bytes)
}

/// Returns the JSON payload of one frame while accepting a trailing partial
/// record for verification and crash-evidence scans.
pub fn trim_jsonl_newline(frame: &[u8]) -> &[u8] {
    frame.strip_suffix(b"\n").unwrap_or(frame)
}

struct CountingWriter {
    bytes: usize,
    limit_bytes: usize,
    overflowed: bool,
}

impl CountingWriter {
    fn new(limit_bytes: usize) -> Self {
        Self {
            bytes: 0,
            limit_bytes,
            overflowed: false,
        }
    }
}

impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let next_bytes = self.bytes.saturating_add(buffer.len());
        if next_bytes > self.limit_bytes {
            self.overflowed = true;
            self.bytes = self.limit_bytes.saturating_add(1);
            return Err(std::io::Error::other(
                "JSONL serialization exceeded its counting limit",
            ));
        }
        self.bytes = next_bytes;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct FixedCapacityWriter {
    bytes: Vec<u8>,
    expected_bytes: usize,
    observed_bytes: usize,
    overflowed: bool,
}

impl FixedCapacityWriter {
    fn new(expected_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(expected_bytes),
            expected_bytes,
            observed_bytes: 0,
            overflowed: false,
        }
    }

    fn write_newline(&mut self) {
        self.observed_bytes = self.observed_bytes.saturating_add(1);
        if self.bytes.len() < self.expected_bytes {
            self.bytes.push(b'\n');
        } else {
            self.overflowed = true;
        }
    }
}

impl Write for FixedCapacityWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.observed_bytes = self.observed_bytes.saturating_add(buffer.len());
        let remaining = self.expected_bytes.saturating_sub(self.bytes.len());
        if buffer.len() > remaining {
            self.overflowed = true;
            return Err(std::io::Error::other(
                "JSONL serialization exceeded its measured fixed capacity",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde::Serializer;

    use super::*;

    #[test]
    fn bounded_encoder_short_circuits_oversize_without_allocating_the_frame() {
        let error = encode_jsonl_frame_bounded(&"12345678", 8).unwrap_err();

        assert!(matches!(
            error,
            BoundedJsonlFrameError::FrameTooLarge {
                observed_at_least_bytes: 9,
                limit_bytes: 8,
            }
        ));
    }

    struct ChangingSerialization {
        calls: AtomicUsize,
    }

    impl Serialize for ChangingSerialization {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                serializer.serialize_str("a")
            } else {
                serializer.serialize_str("longer")
            }
        }
    }

    #[test]
    fn bounded_encoder_rejects_a_stateful_second_pass_without_growing() {
        let error = encode_jsonl_frame_bounded(
            &ChangingSerialization {
                calls: AtomicUsize::new(0),
            },
            64,
        )
        .unwrap_err();

        match error {
            BoundedJsonlFrameError::SizeChanged {
                measured_bytes,
                second_pass_bytes,
            } => {
                assert_eq!(measured_bytes, 4);
                assert!(second_pass_bytes > measured_bytes);
            }
            other => panic!("unexpected bounded encoding error: {other}"),
        }
    }
}

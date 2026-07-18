use super::*;

fn plan(conn_id: &str, private: bool, channel: Channel, symbol: Option<&str>) -> SocketPlan {
    SocketPlan {
        conn_id: ConnId::new(conn_id),
        venue: Venue::Okx,
        private,
        subscriptions: vec![Subscription {
            venue: Venue::Okx,
            channel,
            symbol: symbol.map(str::to_string),
            priority: FeedPriority::Critical,
            connections: 1,
        }],
    }
}

#[test]
fn preparation_resolves_the_secret_free_plan_without_credentials() {
    let mut config = config();
    let missing_prefix = format!("REAP_PHASE1_MISSING_{}", std::process::id());
    config.accounts[0].api_key_env = format!("{missing_prefix}_KEY");
    config.accounts[0].secret_key_env = format!("{missing_prefix}_SECRET");
    config.accounts[0].passphrase_env = format!("{missing_prefix}_PASSPHRASE");
    for name in [
        &config.accounts[0].api_key_env,
        &config.accounts[0].secret_key_env,
        &config.accounts[0].passphrase_env,
    ] {
        assert!(std::env::var_os(name).is_none());
    }

    let prepared = prepare_live(
        config,
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: Some(Duration::from_millis(1)),
        },
    )
    .unwrap();

    assert_eq!(prepared.connectivity_plan().mode(), LiveMode::Observe);
    assert_eq!(prepared.connectivity_plan().sha256().len(), 64);
    assert!(prepared.connectivity_plan().regular_mutations().is_empty());
}

#[test]
fn unsupported_burst_input_fails_preparation_before_credentials() {
    let mut config = config();
    let missing_prefix = format!("REAP_PHASE1_BURST_MISSING_{}", std::process::id());
    config.accounts[0].api_key_env = format!("{missing_prefix}_KEY");
    config.accounts[0].secret_key_env = format!("{missing_prefix}_SECRET");
    config.accounts[0].passphrase_env = format!("{missing_prefix}_PASSPHRASE");
    config.strategy.act_on_burst = true;

    let error = prepare_live(
        config,
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: None,
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        LiveRuntimeError::Config(LiveConfigError::Invalid(ref message))
            if message.contains("strategy.act_on_burst is unsupported by live modes")
    ));
}

#[test]
fn private_account_requires_every_transport_and_state_data_round() {
    let plans = vec![
        plan("orders", true, Channel::Orders, None),
        plan("fills", true, Channel::Fills, None),
        plan("account", true, Channel::Account, None),
        plan("positions", true, Channel::Positions, None),
    ];
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
    let mut source = FeedSourceState::private(adapter, "main".to_string(), &plans);

    assert!(
        source
            .on_status(status("orders", ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(
        source
            .on_status(status("fills", ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(
        source
            .on_status(status("account", ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(
        source
            .on_status(status("positions", ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(
        source
            .on_status(status("orders", ConnectionStatusKind::Heartbeat))
            .is_empty()
    );
    assert!(source.on_private_data(Channel::Account, 2).is_empty());
    assert!(
        source
            .on_status(status("positions", ConnectionStatusKind::Heartbeat))
            .is_empty()
    );
    let ready = source.on_private_data(Channel::Positions, 3);
    assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);

    assert!(source.on_private_data(Channel::Account, 4).is_empty());
    assert!(source.on_private_data(Channel::Account, 5).is_empty());
    assert!(
        source
            .on_status(status("orders", ConnectionStatusKind::Heartbeat))
            .is_empty()
    );
    let heartbeat = source.on_private_data(Channel::Positions, 6);
    assert_eq!(heartbeat[0].kind, SystemEventKind::PrivateStreamHeartbeat);

    let stale = source.on_status(status("fills", ConnectionStatusKind::Disconnected));
    assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
    assert!(stale[0].reason.ends_with(": test"));
    assert!(
        source
            .on_status(status("fills", ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(source.on_private_data(Channel::Positions, 7).is_empty());
    let recovered = source.on_private_data(Channel::Account, 8);
    assert_eq!(recovered[0].kind, SystemEventKind::PrivateStreamRecovered);
}

#[test]
fn one_redundant_book_disconnect_does_not_mark_feed_stale() {
    let plans = vec![
        plan("book-1", false, Channel::Books, Some("BTC-USDT")),
        plan("book-2", false, Channel::Books, Some("BTC-USDT")),
    ];
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
    let mut source = FeedSourceState::public(adapter, &plans);
    source.on_status(status("book-1", ConnectionStatusKind::Ready));
    source.on_status(status("book-2", ConnectionStatusKind::Ready));
    assert_eq!(source.public_connectivity_ready(), Some(true));

    assert!(
        source
            .on_status(status("book-1", ConnectionStatusKind::Disconnected))
            .is_empty()
    );
    assert_eq!(source.public_connectivity_ready(), Some(true));
    let stale = source.on_status(status("book-2", ConnectionStatusKind::Disconnected));
    assert_eq!(source.public_connectivity_ready(), Some(false));
    assert_eq!(stale[0].kind, SystemEventKind::FeedStale);
    assert_eq!(stale[0].symbol.as_deref(), Some("BTC-USDT"));
    assert!(stale[0].reason.ends_with(": test"));
}

#[test]
fn public_plan_materializes_exact_replicas_and_explicit_trades() {
    let mut config = config();
    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
    let subscriptions = runtime_public_subscriptions(&connectivity_plan).unwrap();

    assert!(subscriptions.iter().all(|planned| {
        let expected = if planned.subscription.channel == Channel::Books {
            2
        } else {
            1
        };
        planned.subscription.connections == expected
            && (expected > 1) == planned.redundancy_consumer.is_some()
            && !planned.requirements.is_empty()
    }));
    let configured_symbols = config
        .strategy
        .instruments
        .iter()
        .map(|instrument| instrument.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let trade_symbols = subscriptions
        .iter()
        .filter(|planned| planned.subscription.channel == Channel::Trades)
        .map(|planned| {
            assert_eq!(planned.subscription.connections, 1);
            planned.subscription.symbol.as_deref().unwrap()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(trade_symbols, configured_symbols);
    assert!(subscriptions.iter().any(|subscription| {
        subscription.subscription.channel == Channel::Custom("price-limit".to_string())
            && subscription.subscription.symbol.as_deref() == Some("BTC-USDT")
            && subscription.subscription.priority == FeedPriority::Critical
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription.subscription.channel == Channel::Custom("mark-price".to_string())
            && subscription.subscription.symbol.as_deref() == Some("BTC-PERP")
    }));
    assert!(!subscriptions.iter().any(|subscription| {
        subscription.subscription.channel == Channel::Custom("funding-rate".to_string())
    }));

    config.strategy.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
    assert!(
        runtime_public_subscriptions(&connectivity_plan)
            .unwrap()
            .iter()
            .any(|subscription| {
                subscription.subscription.channel == Channel::Custom("funding-rate".to_string())
                    && subscription.subscription.symbol.as_deref() == Some("BTC-PERP")
                    && subscription.subscription.priority == FeedPriority::Critical
                    && subscription.subscription.connections == 1
            })
    );
}

#[test]
fn public_plan_packs_stablecoin_requirement_without_extra_replica() {
    let mut config = config();
    config.risk.stablecoin_guards = vec![StablecoinGuardConfig {
        symbol: "USDT-USD".to_string(),
        max_downside_deviation: 0.01,
    }];
    config.strategy.instruments[0].index_symbol = Some("USDT-USD".to_string());

    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
    let subscriptions = runtime_public_subscriptions(&connectivity_plan).unwrap();
    let stablecoin = subscriptions
        .iter()
        .filter(|subscription| {
            subscription.subscription.channel == Channel::Custom("index-tickers".to_string())
                && subscription.subscription.symbol.as_deref() == Some("USDT-USD")
        })
        .collect::<Vec<_>>();

    assert_eq!(stablecoin.len(), 1);
    assert_eq!(stablecoin[0].subscription.priority, FeedPriority::Critical);
    assert_eq!(stablecoin[0].subscription.connections, 1);
    assert!(stablecoin[0].requirements.len() >= 2);
}

#[test]
fn private_sessions_are_packed_per_account_and_unused_account_waits_only_for_positions() {
    let mut config = config();
    let mut unused = config.accounts[0].clone();
    unused.id = "unused".to_string();
    unused.api_key_env = "UNUSED_KEY".to_string();
    unused.secret_key_env = "UNUSED_SECRET".to_string();
    unused.passphrase_env = "UNUSED_PASS".to_string();
    unused.id_prefix = "unused".to_string();
    unused.node_id = 2;
    unused.trade_modes.clear();
    config.accounts.push(unused);
    config.venue.enable_vip_fills_channel = true;
    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

    let mut plans = private_socket_plans_by_account(&connectivity_plan).unwrap();
    assert_eq!(plans["main"].len(), 1);
    assert!(
        plans["main"][0]
            .subscriptions
            .iter()
            .any(|subscription| subscription.channel == Channel::Orders)
    );
    let unused = plans.remove("unused").unwrap();
    assert_eq!(unused.len(), 1);
    assert_eq!(unused[0].subscriptions.len(), 1);
    assert_eq!(unused[0].subscriptions[0].channel, Channel::Positions);

    let conn_id = unused[0].conn_id.0.clone();
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
    let mut source = FeedSourceState::private(adapter, "unused".to_string(), &unused);
    assert!(
        source
            .on_status(status(&conn_id, ConnectionStatusKind::Ready))
            .is_empty()
    );
    let ready = source.on_private_data(Channel::Positions, 7);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);
}

#[test]
fn private_session_socket_overcount_is_rejected_by_runtime_composition() {
    let error = validate_private_state_socket_count("main", 2).unwrap_err();
    assert!(
        matches!(
            &error,
            LiveRuntimeError::Subscription(message)
                if message.contains("must use exactly one socket, configured 2")
        ),
        "unexpected private socket overcount error: {error}"
    );
}

#[test]
fn packed_private_session_is_permutation_safe_and_disconnect_resets_its_data_round() {
    let mut config = config();
    config.venue.enable_vip_fills_channel = true;
    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
    let packed = private_socket_plans_by_account(&connectivity_plan)
        .unwrap()
        .remove("main")
        .unwrap();
    assert_eq!(packed.len(), 1);
    assert_eq!(
        packed[0]
            .subscriptions
            .iter()
            .map(|subscription| subscription.channel.clone())
            .collect::<HashSet<_>>(),
        HashSet::from([
            Channel::Orders,
            Channel::Fills,
            Channel::Account,
            Channel::Positions,
        ])
    );
    let conn_id = packed[0].conn_id.0.clone();
    let channels = [
        Channel::Orders,
        Channel::Fills,
        Channel::Account,
        Channel::Positions,
    ];
    let mut permutations = 0;
    for first in 0..channels.len() {
        for second in 0..channels.len() {
            if second == first {
                continue;
            }
            for third in 0..channels.len() {
                if third == first || third == second {
                    continue;
                }
                for fourth in 0..channels.len() {
                    if fourth == first || fourth == second || fourth == third {
                        continue;
                    }
                    permutations += 1;
                    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
                    let mut source = FeedSourceState::private(adapter, "main".to_string(), &packed);
                    assert!(
                        source
                            .on_status(status(&conn_id, ConnectionStatusKind::Ready,))
                            .is_empty()
                    );
                    let mut saw_account = false;
                    let mut saw_positions = false;
                    let mut recovered = false;
                    for (offset, channel) in [first, second, third, fourth]
                        .into_iter()
                        .map(|index| channels[index].clone())
                        .enumerate()
                    {
                        saw_account |= channel == Channel::Account;
                        saw_positions |= channel == Channel::Positions;
                        let events = source.on_private_data(channel, offset as u64 + 2);
                        if saw_account && saw_positions && !recovered {
                            assert_eq!(events.len(), 1);
                            assert_eq!(events[0].kind, SystemEventKind::PrivateStreamRecovered);
                            recovered = true;
                        } else {
                            assert!(events.is_empty());
                        }
                    }
                    assert!(recovered);
                }
            }
        }
    }
    assert_eq!(permutations, 24);

    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
    let mut source = FeedSourceState::private(adapter, "main".to_string(), &packed);
    assert!(
        source
            .on_status(status(&conn_id, ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(source.on_private_data(Channel::Account, 10).is_empty());
    assert_eq!(
        source.on_private_data(Channel::Positions, 11)[0].kind,
        SystemEventKind::PrivateStreamRecovered
    );
    let stale = source.on_status(status(&conn_id, ConnectionStatusKind::Disconnected));
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
    assert!(
        source
            .on_status(status(&conn_id, ConnectionStatusKind::Ready))
            .is_empty()
    );
    assert!(source.on_private_data(Channel::Orders, 12).is_empty());
    assert!(source.on_private_data(Channel::Fills, 13).is_empty());
    assert!(source.on_private_data(Channel::Positions, 14).is_empty());
    let recovered = source.on_private_data(Channel::Account, 15);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].kind, SystemEventKind::PrivateStreamRecovered);
}

#[test]
fn command_sessions_exist_only_for_nonempty_planned_lanes() {
    let mut config = config();
    let mut unused = config.accounts[0].clone();
    unused.id = "unused".to_string();
    unused.api_key_env = "UNUSED_KEY".to_string();
    unused.secret_key_env = "UNUSED_SECRET".to_string();
    unused.passphrase_env = "UNUSED_PASS".to_string();
    unused.id_prefix = "unused".to_string();
    unused.node_id = 2;
    unused.trade_modes.clear();
    config.accounts.push(unused);

    let demo_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
    let counts = planned_order_session_counts(&demo_plan).unwrap();
    assert_eq!(counts, BTreeMap::from([("main".to_string(), 1)]));
    let mut startup =
        StartupGate::new_with_order_transports(&config, counts.keys().cloned().collect()).unwrap();
    assert_eq!(
        startup.snapshot().missing_order_transports,
        vec!["main".to_string()]
    );
    startup
        .mark_order_transport("unused", false, "unplanned account has no lane")
        .unwrap();
    assert_eq!(
        startup.snapshot().missing_order_transports,
        vec!["main".to_string()]
    );
    assert!(
        !startup
            .snapshot()
            .faults
            .contains_key("order_transport:unused")
    );
    let observe_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Observe).unwrap();
    assert!(
        planned_order_session_counts(&observe_plan)
            .unwrap()
            .is_empty()
    );
}

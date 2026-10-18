//! Recovery observations use the ordinary actor/desktop persistence seams. In particular, a
//! channel-read command does not open a document, and no test supplies history to a live peer.
use super::*;

impl Scenario {
    async fn discover_channel(&self, indices: &[usize], channel: u128) {
        bounded("automatic channel directory discovery", async {
            loop {
                let mut complete = true;
                for index in indices {
                    complete &= self
                        .online(*index)
                        .actor
                        .channels()
                        .await
                        .iter()
                        .any(|candidate| candidate.id == channel);
                }
                if complete {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
    }

    /// Compare full device identities, including absence, rather than equating roster counts or
    /// four-byte display fingerprints. An epoch transition must not leave a removed peer present.
    async fn converge_members(&self, indices: &[usize], member_indices: &[usize]) {
        let expected: BTreeSet<_> = member_indices
            .iter()
            .map(|index| hex::encode(self.clients[*index].identity.as_ref().unwrap().1.as_bytes()))
            .collect();
        bounded("exact authenticated membership recovery", async {
            loop {
                let mut complete = true;
                for index in indices {
                    let actual: BTreeSet<_> = self
                        .online(*index)
                        .actor
                        .members()
                        .await
                        .into_iter()
                        .map(|member| member.identity)
                        .collect();
                    complete &= actual == expected;
                }
                if complete {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        self.rules.assert_graph();
    }
}

async fn new_channel_bridge_scenario() {
    let mut scenario = Scenario::new(true);
    scenario.admit().await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    scenario.close(&[0, 1, 2]).await;

    // Only 4 can connect 5 to 6. The directory and channel history must reach that bridge
    // through catch-up, even though neither its UI nor the later receivers select the channel.
    let channel = bounded(
        "create channel during other members' absence",
        scenario
            .online(4)
            .actor
            .create_channel("created while away"),
    )
    .await
    .unwrap();
    assert_ne!(channel.id, channel_id("general"));
    let mut expected = scenario.online(4).history_in(channel.id).await;
    assert!(expected.messages.is_empty());
    scenario
        .author_in(
            channel.id,
            &[4],
            "new channel original author",
            &mut expected,
        )
        .await;
    scenario.discover_channel(&[3, 5], channel.id).await;
    scenario
        .converge_in(
            channel.id,
            &[3, 4, 5],
            &expected,
            "unselected new channel through 4",
        )
        .await;
    scenario.close(&[3, 4, 5]).await;

    // The creator and its other recipient stay offline. Reopen the sole sealed bridge, then
    // members whose saved directories predate creation. Do not inject a catch-up source or call
    // open_channel: production discovery must find both the directory and the original log.
    scenario.reopen(&[3]).await;
    assert_eq!(scenario.online(3).history_in(channel.id).await, expected);
    scenario.reopen(&[0, 1, 2]).await;
    scenario.discover_channel(&[0, 1, 2], channel.id).await;
    scenario
        .converge_in(
            channel.id,
            &[0, 1, 2, 3],
            &expected,
            "sealed bridge onward new-channel service",
        )
        .await;
    scenario
        .author_in(
            channel.id,
            &[1, 2],
            "returning members reply",
            &mut expected,
        )
        .await;
    scenario
        .converge_in(
            channel.id,
            &[0, 1, 2, 3],
            &expected,
            "bidirectional new-channel history",
        )
        .await;
    scenario.close(&[0, 1, 2, 3]).await;

    // Every recipient must have persisted the full exact union independently. Solo reopens
    // provide no network source that could conceal a missing native covering snapshot.
    for index in [0, 1, 2, 3] {
        scenario.reopen(&[index]).await;
        assert_eq!(
            scenario.online(index).history_in(channel.id).await,
            expected
        );
        scenario.close(&[index]).await;
    }
    scenario.rules.assert_graph();
    assert!(scenario.rules.trace.lock().unwrap().dropped_gossip > 0);
}

async fn missed_membership_bridge_scenario() {
    let mut scenario = Scenario::new(true);
    scenario.admit().await;
    let mut expected = History::new();
    // Removed-author semantic history is separately scoped (R6). This matrix uses original
    // operations from continuing members; it tests control recovery before current-epoch data.
    scenario
        .author(&[0, 4], "before membership changes", &mut expected)
        .await;
    scenario
        .converge(ALL, &expected, "pre-change exact history")
        .await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    let removed = scenario.clients[5].identity.as_ref().unwrap().1;
    let notices = [
        scenario.online(1).membership_notices(),
        scenario.online(2).membership_notices(),
    ];
    bounded(
        "owner-authorized native removal",
        scenario
            .online(0)
            .actor
            .remove_member(fingerprint(&removed)),
    )
    .await
    .unwrap();
    scenario
        .converge_members(&[0, 1, 2], &[0, 1, 2, 3, 4])
        .await;
    for (index, before) in [(1, notices[0]), (2, notices[1])] {
        scenario
            .online(index)
            .expect_membership_notices_after(before)
            .await;
    }
    scenario
        .author(&[0, 1], "after membership changes", &mut expected)
        .await;
    scenario
        .converge(&[0, 1, 2], &expected, "current-epoch accepted history")
        .await;
    scenario.close(&[0, 1, 2]).await;

    // The founder stays offline. Member 2 serves the missed control commit to bridge 4; after
    // that source closes, bridge 4 alone must recover surviving member 5 from its stale snapshot.
    scenario.reopen_in(&[1, 3], false).await;
    scenario.converge_members(&[1, 3], &[0, 1, 2, 3, 4]).await;
    scenario
        .online(3)
        .expect_membership_notices_after((0, 0))
        .await;
    scenario
        .converge(&[1, 3], &expected, "non-founder control then chat recovery")
        .await;
    scenario.close(&[1]).await;
    scenario.reopen_in(&[4], false).await;
    scenario.converge_members(&[3, 4], &[0, 1, 2, 3, 4]).await;
    scenario
        .online(4)
        .expect_membership_notices_after((0, 0))
        .await;
    scenario
        .converge(
            &[3, 4],
            &expected,
            "replacement bridge control and original history",
        )
        .await;
    scenario
        .author(&[4], "survivor after missed removal", &mut expected)
        .await;
    scenario
        .converge(&[3, 4], &expected, "surviving member can still author")
        .await;
    for index in [3, 4] {
        assert!(
            !scenario
                .online(index)
                .actor
                .contains_member_device(removed)
                .await
        );
        assert!(scenario
            .online(index)
            .actor
            .member_routes()
            .await
            .iter()
            .all(|route| route.peer_id != Some(*scenario.rules.peers[5].as_bytes())));
    }
    scenario.close(&[3, 4]).await;
    for index in [3, 4] {
        scenario.reopen_in(&[index], false).await;
        scenario.converge_members(&[index], &[0, 1, 2, 3, 4]).await;
        assert_eq!(scenario.online(index).history().await, expected);
        scenario.close(&[index]).await;
    }
    scenario.rules.assert_graph();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_channel_is_discovered_served_without_ui_open_and_sealed_by_every_recipient() {
    tokio::select! {
        _ = new_channel_bridge_scenario() => {},
        _ = SystemClock.sleep(Duration::from_secs(300)) => panic!("new-channel bridge matrix exceeded its total bound"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missed_membership_recovers_before_chat_through_non_founder_and_replacement_bridge() {
    tokio::select! {
        _ = missed_membership_bridge_scenario() => {},
        _ = SystemClock.sleep(Duration::from_secs(300)) => panic!("membership bridge matrix exceeded its total bound"),
    }
}

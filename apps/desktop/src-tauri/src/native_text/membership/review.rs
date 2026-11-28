//! Check one explicitly chosen signed holder on its existing connection.
//! Each native phase returns the original runtime before the TCP request.
use super::*;
use catcoms_app::store::{
    NativeTextFounderReviewResult, NativeTextFounderReviewSweep, NativeTextMemberPreparation,
    V2MemberSaveRefusal,
};
use catcoms_rt::{RequestAuthority, RequestCancellation};
use catcoms_sync::owner_v2::V2RemovedReviewQuery;

// The holder's history quota is 2048 records, at eight entries per page.
const MAX_PAGES: usize = 256;
const UNAVAILABLE: &str = "Could not check this device for posts.";
const REVIEW_TIME_MS: u64 = 60_000;
type Preparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;

#[derive(Clone, Debug)]
pub(super) struct Deadline {
    clock: Arc<dyn Clock>,
    end: u64,
}

impl Deadline {
    pub(super) fn new(clock: Arc<dyn Clock>) -> Self {
        let end = clock.monotonic_ms().saturating_add(REVIEW_TIME_MS);
        Self { clock, end }
    }

    fn expired(&self) -> bool {
        self.clock.monotonic_ms() >= self.end
    }

    fn constrain(&self, caller: RequestCancellation) -> RequestCancellation {
        caller.clone().with_authority(Arc::new(ReviewCall {
            caller,
            deadline: self.clone(),
        }))
    }
}

#[derive(Debug)]
struct ReviewCall {
    caller: RequestCancellation,
    deadline: Deadline,
}

#[async_trait::async_trait]
impl RequestAuthority for ReviewCall {
    fn authorized(&self) -> bool {
        !self.caller.is_cancelled() && !self.deadline.expired()
    }

    async fn revoked(&self) {
        let mut caller = self.caller.clone();
        let remaining = self
            .deadline
            .end
            .saturating_sub(self.deadline.clock.monotonic_ms());
        tokio::select! {
            _ = caller.cancelled() => {}
            _ = self.deadline.clock.sleep(std::time::Duration::from_millis(remaining)) => {}
        }
    }
}

pub(super) async fn check(
    state: &AppState,
    source: &connected::Source,
    admission: OwnerFactId,
    contact: [u8; V2_MEMBER_CONTACT_BYTES],
    deadline: &Deadline,
) -> Result<String, &'static str> {
    let channel = source.channel();
    let original = source.negative();
    if original.is_cancelled() {
        return Err(listing::STALE);
    }
    let caller = deadline.constrain(original);
    let sweep = on_founder(state, source, &caller, move |prepared, caller| {
        prepared.start_founder_removed_review(channel, admission, contact, caller)
    })
    .await?;
    let mut cursor = None;
    let mut held = 0;
    let mut already_in_history = 0;
    let mut unavailable = 0;
    let mut complete = false;
    for _ in 0..MAX_PAGES {
        let result = request(
            state,
            source,
            &caller,
            &sweep,
            V2RemovedReviewQuery::Page(cursor),
        )
        .await;
        let Ok(NativeTextFounderReviewResult::Page(page)) = result else {
            break;
        };
        let Some((actions, offer, continuation)) = page.current_data() else {
            break;
        };
        for &action in actions {
            if caller.is_cancelled() {
                break;
            }
            let query = V2RemovedReviewQuery::Original { offer, action };
            match request(state, source, &caller, &sweep, query).await {
                Ok(NativeTextFounderReviewResult::Held(actual)) if actual == action => {
                    held += usize::from(sweep.count_result_once(actual).map_err(|_| UNAVAILABLE)?);
                }
                Ok(NativeTextFounderReviewResult::AlreadyInHistory(actual)) if actual == action => {
                    already_in_history +=
                        usize::from(sweep.count_result_once(actual).map_err(|_| UNAVAILABLE)?);
                }
                _ if source.negative().is_cancelled() => return Err(listing::STALE),
                _ => unavailable += 1,
            }
        }
        cursor = continuation;
        if caller.is_cancelled() {
            break;
        }
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    if source.negative().is_cancelled() {
        return Err(listing::STALE);
    }
    serde_json::to_string(&serde_json::json!({
        "complete": complete && unavailable == 0,
        "held": held,
        "alreadyInHistory": already_in_history,
        "unavailable": unavailable,
    }))
    .map_err(|_| UNAVAILABLE)
}

async fn request(
    state: &AppState,
    source: &connected::Source,
    caller: &RequestCancellation,
    sweep: &Arc<NativeTextFounderReviewSweep>,
    query: V2RemovedReviewQuery,
) -> Result<NativeTextFounderReviewResult, &'static str> {
    let sweep = sweep.clone();
    let exchange = on_founder(state, source, caller, move |prepared, caller| {
        prepared.prepare_founder_removed_review(sweep, query, caller)
    })
    .await?;
    let reply = exchange.request().await.map_err(|_| UNAVAILABLE)?;
    on_founder(state, source, caller, move |prepared, caller| {
        prepared.receive_founder_removed_review(reply, caller)
    })
    .await
}

async fn on_founder<T: Send + 'static>(
    state: &AppState,
    source: &connected::Source,
    caller: &RequestCancellation,
    run: impl FnOnce(Preparation, &RequestCancellation) -> Result<T, V2MemberSaveRefusal>
        + Send
        + 'static,
) -> Result<T, &'static str> {
    let mut caller = caller.clone();
    if caller.is_cancelled() {
        return Err(UNAVAILABLE);
    }
    let permit = source
        .member_work()
        .foreground(caller.clone())
        .await
        .ok_or(listing::STALE)?;
    let original = tokio::select! {
        biased;
        _ = caller.cancelled() => return Err(UNAVAILABLE),
        captured = source.capture(state) => captured.ok_or(listing::STALE)?,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if caller.is_cancelled() {
            return Err(UNAVAILABLE);
        }
        run(original, &caller).map_err(refusal)
    })
    .await
    .map_err(|_| UNAVAILABLE)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    #[tokio::test]
    async fn review_deadline_starts_before_task_poll_and_preserves_the_original_cancellation() {
        let clock = Arc::new(ManualClock::new(10));
        let deadline = Deadline::new(clock.clone());
        let (owner, live) = tokio::sync::watch::channel(false);
        let source = RequestCancellation::new(live, None);
        let mut in_flight = deadline.constrain(source.clone());
        clock.advance_ms(REVIEW_TIME_MS - 1);
        assert!(!in_flight.is_cancelled());
        let wait = tokio::spawn(async move { in_flight.cancelled().await });
        clock.advance_ms(1);
        wait.await.unwrap();
        // A later producer can only reuse the exact original deadline.
        assert!(deadline.constrain(source.clone()).is_cancelled());
        let fresh = Deadline::new(clock);
        assert!(!fresh.constrain(source.clone()).is_cancelled());
        owner.send_replace(true);
        assert!(fresh.constrain(source).is_cancelled());
    }

    #[tokio::test]
    async fn a_new_limit_preserves_an_earlier_request_authority() {
        let clock = Arc::new(ManualClock::new(10));
        let original = Deadline::new(clock.clone());
        let (_owner, live) = tokio::sync::watch::channel(false);
        let original_signal = original.constrain(RequestCancellation::new(live, None));
        clock.advance_ms(10);
        let later = Deadline::new(clock.clone());
        let mut signal = later.constrain(original_signal);
        clock.advance_ms(REVIEW_TIME_MS - 11);
        assert!(!signal.is_cancelled());
        clock.advance_ms(1);
        assert!(!later.expired());
        assert!(signal.is_cancelled());
        signal.cancelled().await;
    }
}

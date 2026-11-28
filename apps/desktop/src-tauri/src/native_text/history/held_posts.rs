//! Founder-only held-post pages. The renderer receives a short-lived token
//! for the currently published continuation, not its keyed filename.
use super::*;
use catcoms_app::store::{NativeTextHeldReviewContinuation, NativeTextHeldReviewPage};
use catcoms_rt::RngCore;

const CURSOR_MS: u64 = 5 * 60 * 1000;
// Two maximum-sized bodies with sixfold JSON escaping fit within 1 MiB.
const HELD_RESPONSE_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    generation: u64,
    instance: u64,
    server: u64,
    channel: u128,
    context: [u8; 32],
}

impl Scope {
    fn from_invocation(call: &Invocation) -> Self {
        Self {
            generation: call.generation,
            instance: call.instance,
            server: call.read.server,
            channel: call.read.channel,
            context: call.read.context,
        }
    }
}

pub(in crate::native_text) struct Cursor {
    token: [u8; 32],
    scope: Scope,
    expires: u64,
    after: NativeTextHeldReviewContinuation,
}

pub(super) struct Output {
    page: NativeTextHeldReviewPage,
    body: Option<String>,
    next: Option<Cursor>,
    _payment: Arc<Payment>,
}

pub(super) async fn execute(
    state: &AppState,
    call: &Invocation,
    token: Option<[u8; 32]>,
    limit: usize,
) -> Result<Output, String> {
    if call.selection.founder().is_none() {
        return Err("Only the server owner can read held posts.".into());
    }
    let owner = Scope::from_invocation(call);
    let after = if let Some(token) = token {
        let slot = state
            .native_text
            .held_review_cursor
            .try_lock()
            .map_err(|_| BUSY)?;
        let cursor = slot
            .as_ref()
            .filter(|cursor| {
                cursor.token == token
                    && cursor.scope == owner
                    && SystemClock.monotonic_ms() < cursor.expires
            })
            .ok_or(STALE)?;
        Some(cursor.after.clone())
    } else {
        None
    };
    let scope = call.scope.clone();
    let permit = call
        .member_work
        .foreground(scope.clone())
        .await
        .ok_or(STALE)?;
    let Preparation::Founder(founder) = capture(state, call).await? else {
        return Err(STALE.into());
    };
    let channel = call.read.channel;
    let payment = call.payment.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let page = founder
            .audit_founder_held_posts(channel, after.as_ref(), limit, &scope)
            .map_err(|error| match error {
                V2MemberSaveRefusal::Busy => BUSY,
                V2MemberSaveRefusal::Stale | V2MemberSaveRefusal::Cancelled => STALE,
                _ => "Could not verify held posts.",
            })?;
        let next = page.continuation().map(|after| {
            let mut token = [0; 32];
            OsCryptoRng.fill_bytes(&mut token);
            Cursor {
                token,
                scope: owner,
                // Unpublished; its deadline starts at the guarded reply.
                expires: 0,
                after,
            }
        });
        let body = serialize(&page, next.as_ref())?;
        Ok(Output {
            page,
            body: Some(body),
            next,
            _payment: payment,
        })
    })
    .await
    .map_err(|_| STALE.to_owned())?
}

impl Output {
    pub(super) fn resolve<R: tauri::Runtime>(
        &mut self,
        state: &AppState,
        resolver: tauri::ipc::InvokeResolver<R>,
    ) {
        let mut resolver = Some(resolver);
        let _ = self.page.with_publication(|| {
            let Ok(mut slot) = state.native_text.held_review_cursor.try_lock() else {
                return;
            };
            *slot = self.next.take().map(|mut cursor| {
                cursor.expires = SystemClock.monotonic_ms().saturating_add(CURSOR_MS);
                cursor
            });
            drop(slot);
            resolver
                .take()
                .unwrap()
                .resolve(InvokeResponseBody::Json(self.body.take().unwrap()));
        });
        if let Some(resolver) = resolver {
            resolver.reject(STALE);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Post<'a> {
    case_id: String,
    action: String,
    signer: String,
    admission: String,
    original_digest: String,
    removal_header: String,
    removal_epoch: String,
    body: BodyData<'a>,
}

#[derive(Serialize)]
struct Page<'a> {
    backend: &'static str,
    scope: &'static str,
    next: Option<String>,
    items: Option<Vec<Post<'a>>>,
    unavailable: usize,
}

fn serialize(page: &NativeTextHeldReviewPage, next: Option<&Cursor>) -> Result<String, String> {
    page.with_records(|records, unavailable| {
        let items = records.map(|records| {
            records
                .iter()
                .map(|record| Post {
                    case_id: hex::encode(record.case.as_bytes()),
                    action: hex::encode(record.signed.id().as_bytes()),
                    signer: hex::encode(record.signed.signer()),
                    admission: hex::encode(record.admission.as_bytes()),
                    original_digest: hex::encode(record.original.as_bytes()),
                    removal_header: hex::encode(record.removal.0.as_bytes()),
                    removal_epoch: record.removal.1.to_string(),
                    body: action_body(&record.signed),
                })
                .collect()
        });
        encode(
            &Page {
                backend: "text-v2",
                scope: "selected-held-posts",
                next: next.map(|cursor| hex::encode(cursor.token)),
                items,
                unavailable,
            },
            HELD_RESPONSE_LIMIT,
        )
    })
    .map_err(|_| STALE.to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_maximum_escaped_posts_fit_the_held_response_limit() {
        let text = "\u{0001}".repeat(64 * 1024);
        let item = || Post {
            case_id: "01".repeat(32),
            action: "02".repeat(32),
            signer: "03".repeat(32),
            admission: "04".repeat(32),
            original_digest: "05".repeat(32),
            removal_header: "06".repeat(32),
            removal_epoch: u64::MAX.to_string(),
            body: BodyData::Create { text: &text, reply_to: Some("07".repeat(32)) },
        };
        let body = encode(&Page {
            backend: "text-v2",
            scope: "selected-held-posts",
            next: Some("08".repeat(32)),
            items: Some(vec![item(), item()]),
            unavailable: 512,
        }, HELD_RESPONSE_LIMIT).unwrap();
        assert!(body.len() > 2 * 64 * 1024 * 5);
        assert!(body.len() <= HELD_RESPONSE_LIMIT);
    }
}

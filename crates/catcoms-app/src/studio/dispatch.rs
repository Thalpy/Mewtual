//! Typed reply routing inside the single actor custody arm. Document and recovery APIs keep
//! distinct public results; neither invents a current document view for historical content.
use super::*;

pub(crate) enum StudioDispatch {
    Document(Option<StudioRequest>),
    Control(StudioControlRequest),
}
pub(crate) enum StudioResponse {
    Document(Option<StudioView>),
    Control(StudioControlResponse),
}
pub(crate) enum StudioReply {
    Document(oneshot::Sender<Result<Option<StudioView>, String>>),
    Control(oneshot::Sender<Result<StudioControlResponse, String>>),
}
impl StudioReply {
    pub(crate) fn is_closed(&self) -> bool {
        match self {
            Self::Document(reply) => reply.is_closed(),
            Self::Control(reply) => reply.is_closed(),
        }
    }
    pub(crate) async fn closed(&mut self) {
        match self {
            Self::Document(reply) => reply.closed().await,
            Self::Control(reply) => reply.closed().await,
        }
    }
    pub(crate) fn send(self, result: Result<StudioResponse, String>) {
        match self {
            Self::Document(reply) => {
                let _ = reply.send(result.and_then(|r| match r {
                    StudioResponse::Document(view) => Ok(view),
                    _ => Err("mismatched Studio response".into()),
                }));
            }
            Self::Control(reply) => {
                let _ = reply.send(result.and_then(|r| match r {
                    StudioResponse::Control(value) => Ok(value),
                    _ => Err("mismatched Studio control response".into()),
                }));
            }
        }
    }
}

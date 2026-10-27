//! Deterministic ready-source fairness without spawning another mutable owner.
use std::future::{poll_fn, Future};
use std::task::Poll;

pub(super) enum Turn<R, J, C, F, M, S> {
    Reset(R),
    Studio(J),
    Command(C),
    File(F),
    Delivery,
    Maintenance(M),
    Sync(S),
}

// One explicit future per finite owner source keeps polling order visible and testable.
#[allow(clippy::too_many_arguments)]
pub(super) async fn next<R, J, C, F, M, S>(
    cursor: &mut usize,
    reset: impl Future<Output = R>,
    studio: impl Future<Output = J>,
    command: impl Future<Output = C>,
    file: impl Future<Output = F>,
    delivery: impl Future<Output = ()>,
    maintenance: impl Future<Output = M>,
    sync: impl Future<Output = S>,
) -> Turn<R, J, C, F, M, S> {
    tokio::pin!(reset, studio, command, file, delivery, maintenance, sync);
    poll_fn(|cx| {
        // Completed bounded Studio work must precede another native lease. Only owner command
        // turns launch these finite jobs, so draining their completions cannot sustain itself.
        if let Poll::Ready(value) = studio.as_mut().poll(cx) {
            return Poll::Ready(Turn::Studio(value));
        }
        for offset in 0..6 {
            let index = (*cursor + offset) % 6;
            let ready = match index {
                0 => reset.as_mut().poll(cx).map(Turn::Reset),
                1 => command.as_mut().poll(cx).map(Turn::Command),
                2 => file.as_mut().poll(cx).map(Turn::File),
                3 => delivery.as_mut().poll(cx).map(|()| Turn::Delivery),
                4 => maintenance.as_mut().poll(cx).map(Turn::Maintenance),
                5 => sync.as_mut().poll(cx).map(Turn::Sync),
                _ => unreachable!(),
            };
            if ready.is_ready() {
                *cursor = (index + 1) % 6;
                return ready;
            }
        }
        Poll::Pending
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actor_ready_sources_rotate_under_continuous_commands_and_network() {
        let mut cursor = 0;
        let mut observed = Vec::new();
        for _ in 0..12 {
            let turn = next(
                &mut cursor,
                std::future::ready(()),
                std::future::pending::<()>(),
                std::future::ready(()),
                std::future::ready(()),
                std::future::ready(()),
                std::future::ready(()),
                std::future::ready(()),
            )
            .await;
            observed.push(match turn {
                Turn::Reset(()) => 0,
                Turn::Command(()) => 1,
                Turn::File(()) => 2,
                Turn::Delivery => 3,
                Turn::Maintenance(()) => 4,
                Turn::Sync(()) => 5,
                Turn::Studio(()) => unreachable!(),
            });
        }
        assert_eq!(observed, vec![0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5]);
    }
}

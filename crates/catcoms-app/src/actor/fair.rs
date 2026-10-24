//! Deterministic ready-source fairness without spawning another mutable owner.
use std::future::{poll_fn, Future};
use std::task::Poll;

pub(super) enum Turn<R, J, C, F, S> {
    Reset(R),
    Studio(J),
    Command(C),
    File(F),
    Delivery,
    Sync(S),
}

pub(super) async fn next<R, J, C, F, S>(
    cursor: &mut usize,
    reset: impl Future<Output = R>,
    studio: impl Future<Output = J>,
    command: impl Future<Output = C>,
    file: impl Future<Output = F>,
    delivery: impl Future<Output = ()>,
    sync: impl Future<Output = S>,
) -> Turn<R, J, C, F, S> {
    tokio::pin!(reset, studio, command, file, delivery, sync);
    poll_fn(|cx| {
        // Completed bounded Studio work must precede another native lease. Only owner command
        // turns launch these finite jobs, so draining their completions cannot sustain itself.
        if let Poll::Ready(value) = studio.as_mut().poll(cx) {
            return Poll::Ready(Turn::Studio(value));
        }
        for offset in 0..5 {
            let index = (*cursor + offset) % 5;
            let ready = match index {
                0 => reset.as_mut().poll(cx).map(Turn::Reset),
                1 => command.as_mut().poll(cx).map(Turn::Command),
                2 => file.as_mut().poll(cx).map(Turn::File),
                3 => delivery.as_mut().poll(cx).map(|()| Turn::Delivery),
                4 => sync.as_mut().poll(cx).map(Turn::Sync),
                _ => unreachable!(),
            };
            if ready.is_ready() {
                *cursor = (index + 1) % 5;
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
        for _ in 0..10 {
            let turn = next(
                &mut cursor,
                std::future::ready(()),
                std::future::pending::<()>(),
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
                Turn::Sync(()) => 4,
                Turn::Studio(()) => unreachable!(),
            });
        }
        assert_eq!(observed, vec![0, 1, 2, 3, 4, 0, 1, 2, 3, 4]);
    }
}

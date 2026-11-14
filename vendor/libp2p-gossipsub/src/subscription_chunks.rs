// Mewtual compatibility patch to the MIT-licensed upstream library.
// Preserve the original notices; see LICENSE.upstream and MEWTUAL-PATCH.md.
//! Linear partitioning preserving every fitting entry in the hello capture.
//! An oversized singleton is explicitly refused, rather than emitted or hidden.
use crate::{topic::TopicHash, types::RpcOut};
use prost::Message as _;

type Subscription = (TopicHash, bool, bool);
type SizedSubscription = (Subscription, usize);

#[derive(Debug)]
pub(crate) struct OversizedSubscription {
    pub(crate) required: usize,
    pub(crate) allowed: usize,
}

pub(crate) struct SubscriptionChunks {
    input: std::vec::IntoIter<Subscription>,
    pending: Option<Result<SizedSubscription, OversizedSubscription>>,
    budget: usize,
}
impl SubscriptionChunks {
    pub(crate) fn new(input: Vec<Subscription>, budget: usize) -> Self {
        Self {
            input: input.into_iter(),
            pending: None,
            budget,
        }
    }
    fn measure(
        &self,
        (topic, requests_partial, supports_partial): Subscription,
    ) -> Result<SizedSubscription, OversizedSubscription> {
        // Move the SAME String through the canonical protobuf conversion and
        // back. No second topic-string copy is created merely to measure it.
        // A single subscription's RPC length includes its repeated-field key,
        // delimiter and ALL present optionals, including explicitly false flags.
        let mut rpc = RpcOut::Subscribe {
            topic,
            requests_partial,
            supports_partial,
        }
        .into_protobuf();
        let required = rpc.encoded_len();
        if required > self.budget {
            return Err(OversizedSubscription {
                required,
                allowed: self.budget,
            });
        }
        let item = rpc
            .subscriptions
            .pop()
            .expect("closed Subscribe conversion emits one entry");
        let topic = TopicHash::from_raw(item.topic_id.expect("closed Subscribe includes topic"));
        Ok(((topic, requests_partial, supports_partial), required))
    }
}
impl Iterator for SubscriptionChunks {
    type Item = Result<Vec<Subscription>, OversizedSubscription>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut chunk = Vec::new();
        let mut bytes = 0usize;
        loop {
            let next = self
                .pending
                .take()
                .or_else(|| self.input.next().map(|item| self.measure(item)));
            let Some(next) = next else {
                return (!chunk.is_empty()).then_some(Ok(chunk));
            };
            match next {
                Err(error) if chunk.is_empty() => return Some(Err(error)),
                Err(error) => {
                    self.pending = Some(Err(error));
                    return Some(Ok(chunk));
                }
                Ok((item, size)) => {
                    match bytes
                        .checked_add(size)
                        .filter(|total| *total <= self.budget)
                    {
                        Some(total) => {
                            bytes = total;
                            chunk.push(item);
                        }
                        None => {
                            // Each entry already fits by itself. Retain only
                            // this measured next entry; never remeasure it or
                            // scan the growing chunk on every insertion.
                            self.pending = Some(Ok((item, size)));
                            debug_assert!(!chunk.is_empty());
                            return Some(Ok(chunk));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mewtual_hello_chunks_preserve_all_topics_flags_order_and_actual_byte_bounds() {
        for budget in [63, 127, 128, 16384, 65536] {
            let expected: Vec<_> = (0..2000)
                .map(|i| (format!("topic-{i:04}-é"), i % 2 == 0, i % 3 == 0))
                .collect();
            let input = expected
                .iter()
                .map(|(topic, a, b)| (TopicHash::from_raw(topic.clone()), *a, *b))
                .collect();
            let mut observed = Vec::new();
            let mut count = 0;
            for chunk in SubscriptionChunks::new(input, budget) {
                let topics = chunk.unwrap();
                assert!(!topics.is_empty());
                count += 1;
                let rpc = RpcOut::SubscribeMany(topics).into_protobuf();
                assert!(rpc.encoded_len() <= budget);
                observed.extend(rpc.subscriptions.into_iter().map(|entry| {
                    (
                        entry.topic_id.unwrap(),
                        entry.requests_partial.unwrap(),
                        entry.supports_partial.unwrap(),
                    )
                }));
            }
            assert_eq!(observed, expected);
            if budget == 63 {
                assert!(count > 1);
            }
        }
    }
    #[test]
    fn mewtual_hello_exact_singleton_boundary_empty_and_explicit_oversize_do_not_stall() {
        let single = RpcOut::Subscribe {
            topic: TopicHash::from_raw("a"),
            requests_partial: false,
            supports_partial: false,
        }
        .into_protobuf()
        .encoded_len();
        assert!(SubscriptionChunks::new(Vec::new(), 0).next().is_none());
        let mut good =
            SubscriptionChunks::new(vec![(TopicHash::from_raw("a"), false, false)], single);
        assert_eq!(good.next().unwrap().unwrap().len(), 1);
        assert!(good.next().is_none());
        let input = vec![
            (TopicHash::from_raw("a"), false, false),
            (TopicHash::from_raw("too large"), true, false),
            (TopicHash::from_raw("b"), true, true),
        ];
        let mut chunks = SubscriptionChunks::new(input, single);
        assert_eq!(chunks.next().unwrap().unwrap()[0].0.as_str(), "a");
        let error = chunks.next().unwrap().unwrap_err();
        assert!(error.required > error.allowed);
        assert_eq!(chunks.next().unwrap().unwrap()[0].0.as_str(), "b");
        assert!(chunks.next().is_none());
    }
}

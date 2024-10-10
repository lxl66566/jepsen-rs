use std::collections::VecDeque;

use madsim::rand::Rng as _;

use super::{NemesisRecord, NemesisRecordWithAction};

/// The strategy to register and recover nemesis. When a nemesis is executed, it
/// should be put into nemesis register, and at one time, it will be removed
/// from register and resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NemesisRegisterStrategy {
    /// Use a FIFO queue to store and recover the nemesis. `usize` indicates the
    /// maximum size of the queue. when pushing a nemesis into a full queue, the
    /// front nemesis will be dropped, aka. recover. when pushing a nemesis into
    /// a non-full queue, no recover will happen.
    FIFO(usize),

    /// A random queue to store and recover the nemesis. `usize` indicates the
    /// maximum size of the queue. when pushing a nemesis into a full queue, a
    /// random nemesis will be dropped, aka. recover. when pushing a nemesis
    /// into a non-full queue, no recover will happen.
    RandomQueue(usize),
}

impl Default for NemesisRegisterStrategy {
    fn default() -> Self {
        Self::FIFO(1)
    }
}

/// The nemesis register to record nemeses and decide when torecover it.
///
/// Nemeses could overlap, so the register uses a queue to store them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NemesisRegister<T = NemesisRecord> {
    /// The queue of nemeses. The size of queue is same to the current
    /// happenning nemeses count in the cluster.
    queue: VecDeque<T>,

    /// The pop-up strategy of the register queue.
    strategy: NemesisRegisterStrategy,
}

impl<T> NemesisRegister<T> {
    /// Create a new nemesis register
    pub fn new(strategy: NemesisRegisterStrategy) -> Self {
        Self {
            queue: VecDeque::new(),
            strategy,
        }
    }

    /// Set the strategy of the nemesis register
    pub fn with_strategy(mut self, strategy: NemesisRegisterStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Get the pop-up [`NemesisRecord`] by the strategy.
    #[must_use]
    fn pop(&mut self) -> Option<T> {
        match self.strategy {
            NemesisRegisterStrategy::FIFO(n) => (self.queue.len() > n)
                .then(|| self.queue.pop_front())
                .flatten(),
            NemesisRegisterStrategy::RandomQueue(n) => (self.queue.len() > n).then(|| {
                self.queue
                    .remove(madsim::rand::thread_rng().gen_range(0..self.queue.len()))
                    .expect("remove index should be valid")
            }),
        }
    }

    /// Push a [`NemesisRecord`] into the nemesis register, and get the pop-up
    /// result if there is.
    pub fn push(&mut self, record: T) -> Option<T> {
        self.queue.push_back(record);
        self.pop()
    }
}

impl NemesisRegister<NemesisRecord> {
    /// Process an input sequence of nemeses, and return the total sequence of
    /// nemeses execution and recover event.
    ///
    /// This will be used in NemesisGenerator.
    ///
    /// Note: not all nemeses in the input sequence will be recover in the
    /// output. At the end there will be some [`NemesisRecord`] stored in the
    /// queue and will never be pop-up.
    pub fn process(
        &mut self,
        records: impl IntoIterator<Item = NemesisRecord>,
    ) -> Vec<NemesisRecordWithAction> {
        let mut output = Vec::new();
        records.into_iter().for_each(|r| {
            // Output (Recover) previous nemesis first, then Input (Execute) next.
            if let Some(ret) = self.push(r.clone()) {
                output.push(NemesisRecordWithAction::Recover(ret));
            }
            output.push(NemesisRecordWithAction::Execute(r));
        });
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_strategy() {
        // FIFO
        let mut register = NemesisRegister::new(NemesisRegisterStrategy::FIFO(2));
        assert_eq!(register.push(1), None);
        assert_eq!(register.push(2), None);
        assert_eq!(register.push(3), Some(1));
        assert_eq!(register.push(4), Some(2));
        assert_eq!(register.push(5), Some(3));
    }

    #[test]
    fn test_register_process_seq() {
        let mut register = NemesisRegister::new(NemesisRegisterStrategy::FIFO(2));
        let input_seq = [NemesisRecord::Bitflip(0.1), NemesisRecord::Bitflip(0.2)]
            .into_iter()
            .zip(std::iter::repeat(NemesisRecord::Noop))
            .flat_map(|(x, y)| vec![x, y]);
        // the input will be [bitflip(0.1), Noop, bitflip(0.2), Noop]
        assert_eq!(input_seq.clone().count(), 4);
        let output_seq = register.process(input_seq);
        assert_eq!(
            output_seq,
            vec![
                NemesisRecordWithAction::Execute(NemesisRecord::Bitflip(0.1)),
                NemesisRecordWithAction::Execute(NemesisRecord::Noop),
                NemesisRecordWithAction::Recover(NemesisRecord::Bitflip(0.1)),
                NemesisRecordWithAction::Execute(NemesisRecord::Bitflip(0.2)),
                NemesisRecordWithAction::Recover(NemesisRecord::Noop),
                NemesisRecordWithAction::Execute(NemesisRecord::Noop),
            ]
        )
    }
}

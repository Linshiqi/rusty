//! Numbered questions to a server, for answers that are about a moment.
//!
//! The hover card, its fixes and the washed occurrences of a name are each
//! about where the pointer or the caret was and the text as it was, and a
//! server answers when it answers: after the card was put away, after the
//! caret moved on, after a key. Each ask takes the next number and so does
//! every dismissal, and an answer counts only while its number is still the
//! last one taken — the text it is about is the caller's to compare.

use std::cell::Cell;

#[derive(Default)]
pub struct Asks(Cell<u64>);

impl Asks {
    /// A new question: its number, which every earlier one's answer now
    /// falls behind.
    pub fn ask(&self) -> u64 {
        let next = self.0.get() + 1;
        self.0.set(next);
        next
    }

    /// Whatever is waiting on an answer no longer is.
    pub fn forget(&self) {
        self.0.set(self.0.get() + 1);
    }

    /// Whether the answer to question `asked` is still wanted.
    pub fn answers(&self, asked: u64) -> bool {
        asked == self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the last question's answer counts, and none once it was put
    /// away — however the answers arrive.
    #[test]
    fn an_answer_counts_only_while_its_question_is_the_last() {
        let asks = Asks::default();
        let first = asks.ask();
        let second = asks.ask();
        assert!(!asks.answers(first), "overtaken by the second");
        assert!(asks.answers(second));
        asks.forget();
        assert!(!asks.answers(second), "put away before it came");
        let third = asks.ask();
        assert!(asks.answers(third));
        assert!(!asks.answers(first) && !asks.answers(second));
    }
}

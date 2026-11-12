//! Remembers that the user set a task aside, or brought it back.

use super::ports::Snoozes;
use crate::entities::snooze::Snooze;
use crate::entities::time::Timestamp;

/// Replaces what is remembered about `item` by `snooze` (`None`: nothing),
/// and forgets what can no longer hold.
pub fn set_aside(
    store: &dyn Snoozes,
    item: &str,
    snooze: Option<Snooze>,
    now: Timestamp,
) -> Result<(), String> {
    let mut snoozes = store.load();
    snoozes.retain(|known| known.item != item && !known.is_over(now));
    snoozes.extend(snooze);
    store.save(&snoozes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::snooze::Until;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Memory(Mutex<Vec<Snooze>>);

    impl Snoozes for Memory {
        fn load(&self) -> Vec<Snooze> {
            self.0.lock().unwrap().clone()
        }

        fn save(&self, snoozes: &[Snooze]) -> Result<(), String> {
            *self.0.lock().unwrap() = snoozes.to_vec();
            Ok(())
        }
    }

    fn snooze(item: &str, until: Until) -> Snooze {
        Snooze {
            item: item.to_string(),
            until,
            seen: Timestamp(100),
        }
    }

    #[test]
    fn one_memory_per_item_and_none_for_what_is_over() {
        let store = Memory::default();
        let now = Timestamp(1_000);
        set_aside(
            &store,
            "a",
            Some(snooze("a", Until::At(Timestamp(2_000)))),
            now,
        )
        .unwrap();
        set_aside(&store, "b", Some(snooze("b", Until::Moves)), now).unwrap();
        set_aside(&store, "a", Some(snooze("a", Until::Moves)), now).unwrap();
        assert_eq!(
            store.load(),
            [snooze("b", Until::Moves), snooze("a", Until::Moves)]
        );

        // Bringing one back forgets it; time forgets the ones it ended.
        set_aside(
            &store,
            "c",
            Some(snooze("c", Until::At(Timestamp(1_500)))),
            now,
        )
        .unwrap();
        set_aside(&store, "a", None, Timestamp(1_600)).unwrap();
        assert_eq!(store.load(), [snooze("b", Until::Moves)]);
    }
}

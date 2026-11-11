//! Brings the user's desk back, or sets a first one up.

use super::ports::{DeskError, DeskStore};
use crate::entities::desk::Desk;

/// The saved desk; on a first run, one made from the configured sources.
/// A desk that cannot be read is an error, never replaced by a fresh one:
/// the next save would erase what the user wrote.
pub fn open_desk(store: &dyn DeskStore, sources: &[String]) -> Result<Desk, DeskError> {
    Ok(store.load()?.unwrap_or_else(|| Desk::starting(sources)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Shelf(RefCell<Result<Option<Desk>, DeskError>>);

    impl DeskStore for Shelf {
        fn load(&self) -> Result<Option<Desk>, DeskError> {
            self.0.borrow().clone()
        }

        fn save(&self, desk: &Desk) -> Result<(), DeskError> {
            *self.0.borrow_mut() = Ok(Some(desk.clone()));
            Ok(())
        }
    }

    fn sources() -> Vec<String> {
        vec!["gitlab".into(), "github".into()]
    }

    #[test]
    fn a_first_run_starts_from_the_sources() {
        let desk = open_desk(&Shelf(RefCell::new(Ok(None))), &sources()).unwrap();
        assert_eq!(desk.spaces().len(), 3);
    }

    #[test]
    fn a_saved_desk_comes_back_as_it_was() {
        let shelf = Shelf(RefCell::new(Ok(None)));
        let mut desk = Desk::starting(&sources());
        desk.add_space("Perso");
        shelf.save(&desk).unwrap();

        let back = open_desk(&shelf, &sources()).unwrap();
        assert_eq!(back.spaces().len(), 4);
        assert_eq!(back.spaces()[3].name, "Perso");
    }

    #[test]
    fn an_unreadable_desk_is_an_error_not_a_fresh_start() {
        let broken = Shelf(RefCell::new(Err(DeskError::Unreadable("line 3".into()))));
        assert_eq!(
            open_desk(&broken, &sources()),
            Err(DeskError::Unreadable("line 3".into()))
        );
    }
}

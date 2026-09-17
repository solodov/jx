use super::*;
use std::cell::RefCell;

struct FakeTerminal<'a> {
    events: &'a RefCell<Vec<&'static str>>,
    fail_suspend: bool,
}
impl ForegroundTerminal for FakeTerminal<'_> {
    fn suspend(&mut self) -> io::Result<()> {
        self.events.borrow_mut().push("suspend");
        if self.fail_suspend {
            Err(io::Error::other("suspend failed"))
        } else {
            Ok(())
        }
    }
    fn resume(&mut self) -> io::Result<()> {
        self.events.borrow_mut().push("resume");
        Ok(())
    }
}

#[test]
fn foreground_errors_and_cancellation_still_restore_terminal() {
    for fail in [false, true] {
        let events = RefCell::new(Vec::new());
        let mut terminal = FakeTerminal {
            events: &events,
            fail_suspend: false,
        };
        let result = foreground_session(&mut terminal, || {
            events.borrow_mut().extend(["execute", "acknowledge"]);
            if fail {
                Err(io::Error::other("failed or cancelled"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.is_err(), fail);
        assert_eq!(
            *events.borrow(),
            ["suspend", "execute", "acknowledge", "resume"]
        );
    }
}

#[test]
fn failed_suspension_does_not_launch_a_command() {
    let events = RefCell::new(Vec::new());
    let mut terminal = FakeTerminal {
        events: &events,
        fail_suspend: true,
    };
    assert!(foreground_session(&mut terminal, || panic!("must not execute")).is_err());
    assert_eq!(*events.borrow(), ["suspend"]);
}

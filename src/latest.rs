//! The last of several choices made one after another, kept by threads of their own: whichever of them keeps last
//! keeps the last choice made (each kept what was chosen as it started, two choices made at once were kept in the
//! order their threads came to it: a header of the process list clicked, then another, kept the first).

use std::sync::Mutex;

/// A choice made and not yet kept, and the keeping, one at a time.
pub struct Latest<T> {
    asked: Mutex<Option<T>>,
    keeping: Mutex<()>,
}

impl<T> Latest<T> {
    pub const fn new() -> Latest<T> {
        Latest { asked: Mutex::new(None), keeping: Mutex::new(()) }
    }

    /// `choice` made: the one to keep, in place of any made before it and not yet kept.
    pub fn ask(&self, choice: T) {
        *self.asked.lock().unwrap() = Some(choice);
    }

    /// The last choice made, kept by `keep`, where none kept it since (one keeping at a time: a choice taken is kept
    /// before another is taken).
    pub fn keep(&self, keep: impl FnOnce(T)) {
        let _one = self.keeping.lock().unwrap();
        let asked = self.asked.lock().unwrap().take();
        if let Some(choice) = asked {
            keep(choice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_choice_whichever_thread_keeps_last() {
        let latest = Latest::new();
        let kept = Mutex::new(Vec::new());
        // (Two choices made, the thread started for the first keeping after the one started for the second.)
        latest.ask("memory");
        latest.ask("gpu");
        latest.keep(|choice| kept.lock().unwrap().push(choice));
        latest.keep(|choice| kept.lock().unwrap().push(choice));
        assert_eq!(*kept.lock().unwrap(), ["gpu"]);
        // (Made one after another, each kept as it comes.)
        latest.ask("cpu");
        latest.keep(|choice| kept.lock().unwrap().push(choice));
        latest.ask("io");
        latest.keep(|choice| kept.lock().unwrap().push(choice));
        assert_eq!(*kept.lock().unwrap(), ["gpu", "cpu", "io"]);
    }

    #[test]
    fn a_choice_made_while_another_is_kept_is_kept_after_it() {
        let latest = std::sync::Arc::new(Latest::new());
        let kept = std::sync::Arc::new(Mutex::new(Vec::new()));
        let (started, keeping) = std::sync::mpsc::channel();
        let (go, wait) = std::sync::mpsc::channel::<()>();
        latest.ask("memory");
        let first = {
            let (latest, kept) = (latest.clone(), kept.clone());
            std::thread::spawn(move || {
                latest.keep(|choice| {
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                    kept.lock().unwrap().push(choice);
                })
            })
        };
        keeping.recv().unwrap();
        latest.ask("gpu");
        let second = {
            let (latest, kept) = (latest.clone(), kept.clone());
            std::thread::spawn(move || latest.keep(|choice| kept.lock().unwrap().push(choice)))
        };
        go.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(*kept.lock().unwrap(), ["memory", "gpu"]);
    }
}

use indicatif::{ProgressBar, ProgressStyle};

extern crate indicatif;

pub trait Progress: Send + Sync {
    /// Shows progress
    fn progress(&self);
    /// Finishes process
    fn finish(&self);
    /// Sets the current message of the progress
    fn message(&self, message: String);
}

pub struct Progresser {
    bar: ProgressBar,
}

impl Progresser {
    /// Creates a new [`Progresser`].
    ///
    /// # Panics
    ///
    /// Panics if fail to compile output template.
    #[must_use]
    pub fn new(total: u64) -> Self {
        let bar = ProgressBar::new(total);
        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
                )
                .expect("Progress template not parsed")
                .progress_chars("#>-"),
        );

        Self { bar }
    }
}

impl Progress for Progresser {
    fn progress(&self) {
        self.bar.inc(1);
    }

    fn finish(&self) {
        self.bar.finish();
    }

    fn message(&self, message: String) {
        self.bar.set_message(message);
    }
}

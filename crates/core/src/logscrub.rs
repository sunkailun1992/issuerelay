//! Log lines as they are written. A log line can carry Linear's or an executor's text in its
//! fields (`error = %…`: the subscriber escapes only the message and error values), so every
//! line loses its control and invisible characters before it reaches a terminal or a log file,
//! keeping only the newline that ends it. The devhost and the desktop app log through
//! `CleanWriter`.
use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

use crate::text::strip_control;

/// Passes what is written to `W` with `strip_control` applied, keeping only the line ending that
/// closes the text (the last byte, when it is a newline). A log line can then neither move the
/// cursor, change the terminal's state nor forge a second line. The tracing subscriber hands
/// over each line in a single `write_all`; a character split across two writes would show as
/// U+FFFD.
pub struct CleanLines<W>(pub W);

impl<W: Write> Write for CleanLines<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(bytes);
        let (body, ending) = match text.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (&*text, ""),
        };
        let mut clean = strip_control(body);
        clean.push_str(ending);
        self.0.write_all(clean.as_bytes())?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// A `MakeWriter` whose writers pass every line through `CleanLines`: `CleanWriter(io::stderr)`
/// for stderr, `CleanWriter(non_blocking)` for a log file. Colours must be off
/// (`with_ansi(false)`): their escape sequences would be cleaned into stray text.
pub struct CleanWriter<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for CleanWriter<M> {
    type Writer = CleanLines<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        CleanLines(self.0.make_writer())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::Mutex;

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// What `CleanLines` passes on when `text` is written to it in one go.
    fn written(text: &str) -> String {
        let mut out = CleanLines(Vec::new());
        out.write_all(text.as_bytes()).unwrap();
        String::from_utf8(out.0).unwrap()
    }

    #[test]
    fn a_log_line_carries_no_terminal_escapes() {
        // Clear screen, OSC 52 (write the clipboard, ended by BEL), a lone BEL, a carriage return
        // that would overwrite the line, and a right-to-left override.
        let hostile =
            "WARN poll: error=boom\u{1b}[2J\u{1b}]52;c;ZXZpbA==\u{7} more\rrewritten \u{202e}end\n";
        assert_eq!(
            written(hostile),
            "WARN poll: error=boom [2J ]52;c;ZXZpbA==  more rewritten end\n"
        );
    }

    #[test]
    fn only_the_closing_newline_survives() {
        // A newline inside the text would let a field forge a second log line.
        assert_eq!(
            written("INFO a: x=one\nINFO forged: y=two\n"),
            "INFO a: x=one INFO forged: y=two\n"
        );
        assert_eq!(written("\n"), "\n");
        assert_eq!(written("no ending"), "no ending");
        // Text that already ends in a newline before the closing one keeps just one.
        assert_eq!(written("tail\n\n"), "tail \n");
        // Other scripts and punctuation are untouched; so is the C1 range's neighbour U+00A0.
        assert_eq!(written("日本語 é \u{a0}…\n"), "日本語 é \u{a0}…\n");
        // The C1 controls (here NEL and CSI) are controls too.
        assert_eq!(written("a\u{85}b\u{9b}c\n"), "a b c\n");
    }

    /// A writer whose every write fails with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn a_write_reports_the_bytes_it_was_given_and_passes_errors_on() {
        let mut out = CleanLines(Vec::new());
        // `write_all` relies on this: the cleaned text is shorter than what came in.
        assert_eq!(out.write(b"a\x1b[2Jb\n").unwrap(), 7);
        assert_eq!(out.0, b"a [2Jb\n");

        let error = CleanLines(Failing(io::ErrorKind::BrokenPipe))
            .write_all(b"gone\n")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn every_writer_the_subscriber_gets_cleans_its_line() {
        let make = CleanWriter(Mutex::new(Vec::new()));
        make.make_writer()
            .write_all(b"INFO a: x=\x1b]52;c;ZXZpbA==\x07\n")
            .unwrap();
        assert_eq!(*make.0.lock().unwrap(), b"INFO a: x= ]52;c;ZXZpbA== \n");
    }
}

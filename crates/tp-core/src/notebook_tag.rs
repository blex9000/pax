//! Parses the info-string of a fenced markdown code block to detect a
//! "notebook cell" — a block tagged for execution by the Markdown panel
//! (e.g. ```` ```python run ```` or ```` ```bash watch=5s confirm ````).
//!
//! Pure logic, no GTK / no I/O — fully unit-testable in `pax-core`.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    Bash,
    Sh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecMode {
    /// One-shot execution: tag was `run` or `once`. Manual trigger.
    Once,
    /// Cyclic execution every `interval`. Auto-start when panel visible.
    Watch { interval: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCellSpec {
    pub lang: Lang,
    pub mode: ExecMode,
    pub timeout: Option<Duration>,
    pub confirm: bool,
}

impl NotebookCellSpec {
    /// Returns `Some(spec)` if `info` is a notebook-cell info string,
    /// `None` otherwise (in which case the block is rendered as a normal
    /// code block).
    pub fn parse(info: &str) -> Option<Self> {
        let mut tokens = info.split_whitespace();
        let lang = match tokens.next()? {
            "python" => Lang::Python,
            "bash" => Lang::Bash,
            "sh" => Lang::Sh,
            _ => return None,
        };
        let mode_tok = tokens.next()?;
        let mode = parse_mode(mode_tok)?;
        let mut timeout = None;
        let mut confirm = false;
        for tok in tokens {
            if tok == "confirm" {
                confirm = true;
            } else if let Some(rest) = tok.strip_prefix("timeout=") {
                timeout = Some(parse_duration(rest)?);
            } else {
                return None;
            }
        }
        Some(NotebookCellSpec { lang, mode, timeout, confirm })
    }
}

fn parse_mode(tok: &str) -> Option<ExecMode> {
    match tok {
        "run" | "once" => Some(ExecMode::Once),
        _ => {
            if let Some(rest) = tok.strip_prefix("watch=") {
                Some(ExecMode::Watch { interval: parse_duration(rest)? })
            } else {
                None
            }
        }
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let (num, mul) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else {
        return None;
    };
    let n: u64 = num.parse().ok()?;
    Some(Duration::from_millis(n * mul))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_code_block_returns_none() {
        assert!(NotebookCellSpec::parse("python").is_none());
        assert!(NotebookCellSpec::parse("rust").is_none());
        assert!(NotebookCellSpec::parse("").is_none());
    }

    #[test]
    fn run_python_minimal() {
        let s = NotebookCellSpec::parse("python run").unwrap();
        assert_eq!(s.lang, Lang::Python);
        assert_eq!(s.mode, ExecMode::Once);
        assert!(s.timeout.is_none());
        assert!(!s.confirm);
    }

    #[test]
    fn once_is_alias_of_run() {
        let s1 = NotebookCellSpec::parse("python run").unwrap();
        let s2 = NotebookCellSpec::parse("python once").unwrap();
        assert_eq!(s1.mode, s2.mode);
    }

    #[test]
    fn watch_interval_seconds() {
        let s = NotebookCellSpec::parse("bash watch=5s").unwrap();
        assert_eq!(s.lang, Lang::Bash);
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_secs(5) });
    }

    #[test]
    fn watch_interval_minutes_and_ms() {
        let s = NotebookCellSpec::parse("sh watch=2m").unwrap();
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_secs(120) });
        let s = NotebookCellSpec::parse("sh watch=500ms").unwrap();
        assert_eq!(s.mode, ExecMode::Watch { interval: Duration::from_millis(500) });
    }

    #[test]
    fn timeout_attribute() {
        let s = NotebookCellSpec::parse("python run timeout=120s").unwrap();
        assert_eq!(s.timeout, Some(Duration::from_secs(120)));
    }

    #[test]
    fn confirm_attribute() {
        let s = NotebookCellSpec::parse("python watch=2s confirm").unwrap();
        assert!(s.confirm);
    }

    #[test]
    fn attribute_order_is_free() {
        let a = NotebookCellSpec::parse("python run timeout=10s confirm").unwrap();
        let b = NotebookCellSpec::parse("python run confirm timeout=10s").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_attribute_returns_none() {
        assert!(NotebookCellSpec::parse("python run weird=1").is_none());
        assert!(NotebookCellSpec::parse("python run --flag").is_none());
    }

    #[test]
    fn unknown_lang_returns_none() {
        assert!(NotebookCellSpec::parse("ruby run").is_none());
    }

    #[test]
    fn missing_mode_returns_none() {
        assert!(NotebookCellSpec::parse("python").is_none());
        assert!(NotebookCellSpec::parse("python timeout=5s").is_none());
    }

    #[test]
    fn malformed_duration_returns_none() {
        assert!(NotebookCellSpec::parse("python watch=").is_none());
        assert!(NotebookCellSpec::parse("python watch=abc").is_none());
        assert!(NotebookCellSpec::parse("python watch=10").is_none());
    }
}

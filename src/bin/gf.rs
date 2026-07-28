use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, LineWriter, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

const ESC: u8 = 0x1b;
const HELP: &str = "\
usage: gf [OPTION]... [GREP-ARG]...
       <producer> | gf [OPTION]...

Folds grep-style output: a file path repeated on consecutive lines is printed
once, and configured directory prefixes are removed from the paths shown.

  --strip PREFIX   drop PREFIX from paths (repeatable; also GF_STRIP, ':'-separated)
  --no-base        do not print the 'base: PREFIX' line announcing a strip
  --cmd PROG       child to run instead of grep (also GF_CMD)
  --stdin          filter stdin even if arguments follow
  --               end gf options; everything after goes to the child
  --help, --version

With no GREP-ARG, gf filters stdin. With GREP-ARG, gf runs the child, filters
its stdout, and exits with the child's status. $PWD is always stripped.
";

enum Detected {
    /// Same path as the previous line: drop the whole path.
    Fold(usize),
    /// New path: drop this many leading bytes of it.
    New { strip_len: usize },
}

struct Folder<F> {
    exists: F,
    strips: Vec<Vec<u8>>,
    show_base: bool,
    bases_shown: Vec<Vec<u8>>,
    last_path: Option<Vec<u8>>,
    known: HashSet<Vec<u8>>,
    misses: HashSet<Vec<u8>>,
    /// Once a `path:12:` line has been seen, lines without a line number are
    /// content, so guessing paths on them only wastes stat() calls.
    lineno_mode: bool,
    logical: Vec<u8>,
}

impl<F: FnMut(&[u8]) -> bool> Folder<F> {
    fn new(mut strips: Vec<Vec<u8>>, show_base: bool, exists: F) -> Self {
        strips.sort_by_key(|p| std::cmp::Reverse(p.len()));
        Self {
            exists,
            strips,
            show_base,
            bases_shown: Vec::new(),
            last_path: None,
            known: HashSet::new(),
            misses: HashSet::new(),
            lineno_mode: false,
            logical: Vec::new(),
        }
    }

    fn line(&mut self, raw: &[u8], out: &mut dyn Write) -> io::Result<()> {
        let mut buf = std::mem::take(&mut self.logical);
        let logical: &[u8] = if raw.contains(&ESC) {
            buf.clear();
            strip_ansi_into(raw, &mut buf);
            &buf
        } else {
            raw
        };

        let drop_len = match self.detect(logical) {
            None => 0,
            Some(Detected::Fold(path_len)) => path_len,
            Some(Detected::New { strip_len }) => {
                if strip_len > 0 && self.show_base {
                    let base = &logical[..strip_len];
                    if !self
                        .bases_shown
                        .iter()
                        .any(|b| b == base)
                    {
                        self.bases_shown
                            .push(base.to_vec());
                        out.write_all(b"base: ")?;
                        out.write_all(base)?;
                        out.write_all(b"\n")?;
                    }
                }
                strip_len
            }
        };

        self.logical = buf;
        write_trimmed(raw, drop_len, out)
    }

    fn detect(&mut self, logical: &[u8]) -> Option<Detected> {
        if let Some(last) = &self.last_path {
            if logical.len() > last.len()
                && logical.starts_with(last)
                && matches!(logical[last.len()], b':' | b'-')
            {
                return Some(Detected::Fold(last.len()));
            }
        }

        let mut end = None;
        for i in separators(logical) {
            if lineno_follows(logical, i) && self.is_path(&logical[..i]) {
                self.lineno_mode = true;
                end = Some(i);
                break;
            }
        }
        if end.is_none() && !self.lineno_mode {
            // `grep -l`, `grep -c` and match lines without -n have no line
            // number to anchor on; a space rules out most content lines.
            for i in separators(logical) {
                if !logical[..i].contains(&b' ') && self.is_path(&logical[..i]) {
                    end = Some(i);
                    break;
                }
            }
        }
        let end = end?;

        let path = &logical[..end];
        let strip_len = self
            .strips
            .iter()
            .find(|p| path.starts_with(p) && path.len() > p.len())
            .map_or(0, |p| p.len());
        self.last_path = Some(path.to_vec());
        Some(Detected::New { strip_len })
    }

    fn is_path(&mut self, cand: &[u8]) -> bool {
        if cand.is_empty() {
            return false;
        }
        if self
            .known
            .contains(cand)
        {
            return true;
        }
        if self
            .misses
            .contains(cand)
        {
            return false;
        }
        if (self.exists)(cand) {
            self.known
                .insert(cand.to_vec());
            true
        } else {
            if self
                .misses
                .len()
                >= 8192
            {
                self.misses
                    .clear();
            }
            self.misses
                .insert(cand.to_vec());
            false
        }
    }
}

/// Candidate path/line-number delimiters, capped so a line dense in `-` or `:`
/// cannot turn into a stat() storm.
fn separators(logical: &[u8]) -> impl Iterator<Item = usize> + '_ {
    logical
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, b)| matches!(b, b':' | b'-'))
        .map(|(i, _)| i)
        .take(64)
}

fn lineno_follows(logical: &[u8], i: usize) -> bool {
    let sep = logical[i];
    let mut j = i + 1;
    while j < logical.len() && logical[j].is_ascii_digit() {
        j += 1;
    }
    j > i + 1 && j < logical.len() && logical[j] == sep
}

fn ansi_end(raw: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    if raw.get(i) == Some(&b'[') {
        i += 1;
        while i < raw.len() {
            if (0x40..=0x7e).contains(&raw[i]) {
                return i + 1;
            }
            i += 1;
        }
        return raw.len();
    }
    (start + 2).min(raw.len())
}

fn strip_ansi_into(raw: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == ESC {
            i = ansi_end(raw, i);
        } else {
            let start = i;
            while i < raw.len() && raw[i] != ESC {
                i += 1;
            }
            out.extend_from_slice(&raw[start..i]);
        }
    }
}

/// Writes `raw` minus its first `drop` printable bytes, keeping every escape
/// sequence so colored output stays colored.
fn write_trimmed(raw: &[u8], drop: usize, out: &mut dyn Write) -> io::Result<()> {
    let mut i = 0;
    let mut seen = 0;
    while i < raw.len() {
        if raw[i] == ESC {
            let end = ansi_end(raw, i);
            out.write_all(&raw[i..end])?;
            i = end;
        } else {
            let start = i;
            while i < raw.len() && raw[i] != ESC {
                i += 1;
            }
            let run = &raw[start..i];
            if seen >= drop {
                out.write_all(run)?;
            } else if seen + run.len() > drop {
                out.write_all(&run[drop - seen..])?;
            }
            seen += run.len();
        }
    }
    out.write_all(b"\n")
}

struct Opts {
    strips: Vec<Vec<u8>>,
    show_base: bool,
    cmd: OsString,
    stdin: bool,
    child_args: Vec<OsString>,
}

fn parse_args() -> Result<Option<Opts>, String> {
    let mut strips: Vec<Vec<u8>> = Vec::new();
    let mut show_base = true;
    let mut stdin = false;
    let mut cmd = env::var_os("GF_CMD").unwrap_or_else(|| OsString::from("grep"));
    let mut child_args = Vec::new();
    let mut args = env::args_os().skip(1);

    while let Some(arg) = args.next() {
        let flag = arg
            .to_str()
            .unwrap_or("")
            .to_owned();
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (flag.as_str(), None),
        };
        match name {
            "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--version" => {
                println!("gf {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--no-base" => show_base = false,
            "--stdin" => stdin = true,
            "--strip" | "--cmd" => {
                let value = match inline {
                    Some(v) => OsString::from(v),
                    None => args
                        .next()
                        .ok_or_else(|| format!("{name} requires an argument"))?,
                };
                if name == "--strip" {
                    strips.push(with_slash(value.as_bytes()));
                } else {
                    cmd = value;
                }
            }
            "--" => {
                child_args.extend(args);
                break;
            }
            _ => {
                child_args.push(arg);
                child_args.extend(args);
                break;
            }
        }
    }

    if let Some(env_strips) = env::var_os("GF_STRIP") {
        for p in env_strips
            .as_bytes()
            .split(|b| *b == b':')
        {
            if !p.is_empty() {
                strips.push(with_slash(p));
            }
        }
    }
    if let Ok(pwd) = env::current_dir() {
        strips.push(with_slash(
            pwd.as_os_str()
                .as_bytes(),
        ));
    }

    Ok(Some(Opts {
        strips,
        show_base,
        cmd,
        stdin,
        child_args,
    }))
}

fn with_slash(prefix: &[u8]) -> Vec<u8> {
    let mut p = prefix.to_vec();
    if !p.ends_with(b"/") {
        p.push(b'/');
    }
    p
}

fn pump(
    mut input: impl BufRead,
    out: &mut dyn Write,
    folder: &mut Folder<impl FnMut(&[u8]) -> bool>,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        folder.line(&line, out)?;
    }
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(Some(o)) => o,
        Ok(None) => return ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("gf: {msg}");
            return ExitCode::from(2);
        }
    };

    let mut folder = Folder::new(opts.strips, opts.show_base, |p: &[u8]| {
        Path::new(OsStr::from_bytes(p)).exists()
    });
    let stdout = io::stdout();
    let mut out: Box<dyn Write> = if stdout.is_terminal() {
        Box::new(LineWriter::new(stdout.lock()))
    } else {
        Box::new(BufWriter::with_capacity(64 * 1024, stdout.lock()))
    };

    let run_child = !opts
        .child_args
        .is_empty()
        && !opts.stdin;
    let mut child = if run_child {
        match Command::new(&opts.cmd)
            .args(&opts.child_args)
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!(
                    "gf: cannot run {}: {e}",
                    opts.cmd
                        .to_string_lossy()
                );
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };

    let result = match &mut child {
        Some(c) => {
            let stdout = c
                .stdout
                .take()
                .expect("piped");
            pump(
                BufReader::with_capacity(64 * 1024, stdout),
                &mut out,
                &mut folder,
            )
        }
        None => pump(io::stdin().lock(), &mut out, &mut folder),
    };
    let flushed = out.flush();

    for e in [result, flushed] {
        if let Err(e) = e {
            if e.kind() == io::ErrorKind::BrokenPipe {
                return ExitCode::from(141);
            }
            eprintln!("gf: {e}");
            return ExitCode::from(2);
        }
    }

    match child
        .as_mut()
        .map(|c| c.wait())
    {
        None => ExitCode::SUCCESS,
        Some(Ok(status)) => match (status.code(), status.signal()) {
            (Some(code), _) => ExitCode::from(code as u8),
            (None, Some(sig)) => ExitCode::from(128u8.wrapping_add(sig as u8)),
            (None, None) => ExitCode::from(2),
        },
        Some(Err(e)) => {
            eprintln!(
                "gf: waiting for {}: {e}",
                opts.cmd
                    .to_string_lossy()
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(lines: &[&str], strips: &[&str], paths: &[&str]) -> String {
        let known: HashSet<Vec<u8>> = paths
            .iter()
            .map(|p| {
                p.as_bytes()
                    .to_vec()
            })
            .collect();
        let mut folder = Folder::new(
            strips
                .iter()
                .map(|s| with_slash(s.as_bytes()))
                .collect(),
            true,
            move |p: &[u8]| known.contains(p),
        );
        let mut out = Vec::new();
        for line in lines {
            folder
                .line(line.as_bytes(), &mut out)
                .unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    const CFG: &str =
        "/mnt/GIT/ng911/rust/test-data/deploy-configs/localhost/noans-worker-lab/config.yaml";

    #[test]
    fn folds_context_lines_and_strips_base() {
        let lines = [
            format!("{CFG}:44:  notify_command:"),
            format!("{CFG}-45-    endpoint:"),
            format!("{CFG}-46-      loopback:"),
        ];
        let lines: Vec<&str> = lines
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            fold(&lines, &["/mnt/GIT"], &[CFG]),
            "base: /mnt/GIT/\n\
             ng911/rust/test-data/deploy-configs/localhost/noans-worker-lab/config.yaml:44:  notify_command:\n\
             -45-    endpoint:\n\
             -46-      loopback:\n"
        );
    }

    #[test]
    fn second_file_prints_its_path_again() {
        let out = fold(
            &[
                "/a/one.rs:1:x",
                "/a/one.rs-2-y",
                "/a/two.rs:9:z",
                "/a/two.rs-10-w",
            ],
            &["/a"],
            &["/a/one.rs", "/a/two.rs"],
        );
        assert_eq!(out, "base: /a/\none.rs:1:x\n-2-y\ntwo.rs:9:z\n-10-w\n");
    }

    #[test]
    fn unknown_paths_and_separators_pass_through() {
        let out = fold(
            &["--", "Binary file /a/x matches", "/b/no.rs:1:x"],
            &[],
            &[],
        );
        assert_eq!(out, "--\nBinary file /a/x matches\n/b/no.rs:1:x\n");
    }

    #[test]
    fn dashes_in_content_are_not_mistaken_for_a_path() {
        let out = fold(
            &["/a/f.rs:1:let x = 1-2-3;", "/a/f.rs-2-// see a-1-b"],
            &[],
            &["/a/f.rs"],
        );
        assert_eq!(out, "/a/f.rs:1:let x = 1-2-3;\n-2-// see a-1-b\n");
    }

    #[test]
    fn no_line_numbers_still_folds_and_strips() {
        let out = fold(&["/a/x.rs:hit", "/a/x.rs:another"], &["/a"], &["/a/x.rs"]);
        assert_eq!(out, "base: /a/\nx.rs:hit\n:another\n");
    }

    #[test]
    fn colored_output_keeps_escapes() {
        let colored = "\x1b[35m\x1b[K/a/x.rs\x1b[m\x1b[K\x1b[36m\x1b[K:\x1b[m\x1b[K7:hit";
        let out = fold(&[colored, "/a/x.rs-8-next"], &["/a"], &["/a/x.rs"]);
        assert_eq!(
            out,
            "base: /a/\n\x1b[35m\x1b[Kx.rs\x1b[m\x1b[K\x1b[36m\x1b[K:\x1b[m\x1b[K7:hit\n-8-next\n"
        );
    }

    #[test]
    fn path_equal_to_strip_prefix_is_kept() {
        let out = fold(&["/a/b:1:x"], &["/a/b"], &["/a/b"]);
        assert_eq!(out, "/a/b:1:x\n");
    }
}

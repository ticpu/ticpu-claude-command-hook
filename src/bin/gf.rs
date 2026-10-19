//! Folds grep-style output: a path repeated on consecutive lines prints once, and
//! known prefixes are stripped.
//!
//! A candidate reaches the filesystem only once it is path-shaped: source lines
//! open with text that names a file in the tree, and accepting one eats that
//! text, the one failure the fold must not have. A wrong rejection only prints
//! the path in full, so the test stays conservative. A slash is sufficient but
//! not required, matches in the working directory having none. The fast path
//! reusing the previous line's path goes through the same test.
//!
//! Under `--git`, a candidate missing from disk may be `git grep`'s `<rev>:path`.
//! It is split at its first colon and accepted only when `git cat-file` finds a
//! blob there, the path resolved from the working directory as git grep prints
//! it; the `<rev>:` part is then stripped and announced like a base. A rev with
//! a colon of its own (`HEAD:dir`) is never split right and prints in full.

use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, LineWriter, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, ExitCode, Stdio};

const ESC: u8 = 0x1b;
const HELP: &str = "\
usage: gf [OPTION]... [GREP-ARG]...
       <producer> | gf [OPTION]...

Folds grep-style output: a file path repeated on consecutive lines is printed
once, and configured directory prefixes are removed from the paths shown.

  --strip PREFIX   drop PREFIX from paths (repeatable; also GF_STRIP, ':'-separated)
  --no-base        do not print the 'base: PREFIX' line announcing a strip
  --git            also fold git grep's REV:PATH, checked with git cat-file
  --cmd PROG       child to run instead of grep (also GF_CMD)
  --stdin          filter stdin even if arguments follow
  --               end gf options; everything after goes to the child
  --help, --version

With no GREP-ARG, gf filters stdin. With GREP-ARG, gf runs the child, filters
its stdout, and exits with the child's status. $PWD is always stripped.

Running the child, gf also reads its stderr and counts the diagnostics that
repeat: 'N × Permission denied (e.g. PATH)' on gf's own stderr, one line per
kind. The two streams stay separate, so nothing is interleaved or hidden.
";

/// One searcher diagnostic, split into the part that repeats and the part that
/// does not. `rg` reports every unreadable path, which on a tree like /var/log is
/// the same sentence a thousand times over with only the path moving.
struct Diagnostic<'a> {
    kind: &'a str,
    path: &'a str,
}

/// Splits `rg: /var/log/x: Permission denied (os error 13)` and the ugrep and GNU
/// grep spellings of it into kind and path. `None` for a line that is not one of
/// those, which then prints verbatim.
fn parse_diagnostic(line: &str) -> Option<Diagnostic<'_>> {
    let rest = line
        .split_once(": ")
        .map(|(prog, rest)| {
            if prog.contains('/') || prog.contains(' ') {
                line
            } else {
                rest
            }
        })?;
    let rest = rest
        .strip_prefix("warning: ")
        .unwrap_or(rest);
    let rest = rest
        .strip_prefix("cannot read ")
        .unwrap_or(rest);
    let (path, kind) = rest.rsplit_once(": ")?;
    let kind = kind
        .split(" (os error ")
        .next()?
        .trim_end();
    (!path.is_empty() && !kind.is_empty() && !kind.contains('/'))
        .then_some(Diagnostic { kind, path })
}

/// Counts each diagnostic kind and keeps the first path that showed it, so the
/// summary carries something to act on without repeating the rest.
#[derive(Default)]
struct Errors {
    kinds: Vec<(String, String, usize)>,
    passthrough: Vec<String>,
}

impl Errors {
    fn line(&mut self, line: &str) {
        match parse_diagnostic(line) {
            Some(Diagnostic { kind, path }) => {
                match self
                    .kinds
                    .iter_mut()
                    .find(|(k, _, _)| k == kind)
                {
                    Some((_, _, n)) => *n += 1,
                    None => self
                        .kinds
                        .push((kind.to_string(), path.to_string(), 1)),
                }
            }
            None => self
                .passthrough
                .push(line.to_string()),
        }
    }

    /// A single occurrence is its own line: a count of one buys nothing and loses
    /// the path the searcher already named.
    fn report(&self, out: &mut dyn Write) -> io::Result<()> {
        for line in &self.passthrough {
            writeln!(out, "{line}")?;
        }
        for (kind, path, n) in &self.kinds {
            if *n == 1 {
                writeln!(out, "gf: {kind}: {path}")?;
            } else {
                writeln!(out, "gf: {n} × {kind} (e.g. {path})")?;
            }
        }
        Ok(())
    }
}

enum Detected {
    /// Same path as the previous line: drop the whole path.
    Fold(usize),
    /// New path: drop this many leading bytes of it.
    New { strip_len: usize },
}

/// Finds a candidate path: `Some(0)` on disk, `Some(n)` in git under an
/// `n`-byte `<rev>:` prefix.
struct Folder<F> {
    lookup: F,
    strips: Vec<Vec<u8>>,
    show_base: bool,
    bases_shown: Vec<Vec<u8>>,
    last_path: Option<Vec<u8>>,
    known: HashMap<Vec<u8>, usize>,
    misses: HashSet<Vec<u8>>,
    /// Once a `path:12:` line has been seen, lines without a line number are
    /// content, so guessing paths on them only wastes stat() calls.
    lineno_mode: bool,
    logical: Vec<u8>,
    seps: Vec<usize>,
}

impl<F: FnMut(&[u8]) -> Option<usize>> Folder<F> {
    fn new(mut strips: Vec<Vec<u8>>, show_base: bool, lookup: F) -> Self {
        strips.sort_by_key(|p| std::cmp::Reverse(p.len()));
        Self {
            lookup,
            strips,
            show_base,
            bases_shown: Vec::new(),
            last_path: None,
            known: HashMap::new(),
            misses: HashSet::new(),
            lineno_mode: false,
            logical: Vec::new(),
            seps: Vec::new(),
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
                && is_path_shaped(last)
            {
                return Some(Detected::Fold(last.len()));
            }
        }

        // Longest candidate first: a directory that is a prefix of the real
        // path also stats fine (`GIT/freeswitch` inside `GIT/freeswitch-esl-…`),
        // and accepting it poisons `last_path` for every following line.
        let mut seps = std::mem::take(&mut self.seps);
        seps.clear();
        seps.extend(separators(logical));

        let mut found = None;
        for &i in seps
            .iter()
            .rev()
        {
            if !lineno_follows(logical, i) {
                continue;
            }
            if let Some(rev_len) = self.is_path(&logical[..i]) {
                self.lineno_mode = true;
                found = Some((i, rev_len));
                break;
            }
        }
        if found.is_none() && !self.lineno_mode {
            // `grep -l`, `grep -c` and match lines without -n have no line
            // number to anchor on; under -l the whole line is the path, and a
            // space rules out the content lines a slash lets past the shape test.
            for i in std::iter::once(logical.len()).chain(
                seps.iter()
                    .rev()
                    .copied(),
            ) {
                if logical[..i].contains(&b' ') {
                    continue;
                }
                if let Some(rev_len) = self.is_path(&logical[..i]) {
                    found = Some((i, rev_len));
                    break;
                }
            }
        }
        self.seps = seps;
        let (end, rev_len) = found?;

        let path = &logical[..end];
        let strip_len = if rev_len > 0 {
            rev_len
        } else {
            self.strips
                .iter()
                .find(|p| path.starts_with(p) && path.len() > p.len())
                .map_or(0, |p| p.len())
        };
        self.last_path = Some(path.to_vec());
        Some(Detected::New { strip_len })
    }

    fn is_path(&mut self, cand: &[u8]) -> Option<usize> {
        if !is_path_shaped(cand) {
            return None;
        }
        if let Some(&rev_len) = self
            .known
            .get(cand)
        {
            return Some(rev_len);
        }
        if self
            .misses
            .contains(cand)
        {
            return None;
        }
        if let Some(rev_len) = (self.lookup)(cand) {
            self.known
                .insert(cand.to_vec(), rev_len);
            Some(rev_len)
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
            None
        }
    }
}

/// The `Folder` lookup: on disk first, then in git when `--git` gave a `CatFile`.
fn find(cand: &[u8], cat_file: Option<&mut CatFile>) -> Option<usize> {
    if Path::new(OsStr::from_bytes(cand)).exists() {
        return Some(0);
    }
    cat_file?.lookup(cand)
}

/// Splits git grep's `<rev>:path` at its first colon. An absolute path cannot
/// name a tree entry, so it is never split.
fn split_rev(cand: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = cand
        .iter()
        .position(|b| *b == b':')?;
    let (rev, path) = (&cand[..colon], &cand[colon + 1..]);
    (!rev.is_empty() && !path.is_empty() && !path.starts_with(b"/")).then_some((rev, path))
}

/// One `git cat-file --batch-check` for the whole run, so a path costs a pipe
/// round trip instead of a process. Started on first use; once it fails, the
/// failure is reported and every later lookup misses.
#[derive(Default)]
struct CatFile {
    pipe: Option<(ChildStdin, BufReader<ChildStdout>)>,
    failed: bool,
}

impl CatFile {
    /// The `<rev>:` length when `cand` is a blob in git.
    fn lookup(&mut self, cand: &[u8]) -> Option<usize> {
        let (rev, path) = split_rev(cand)?;
        if self.failed {
            return None;
        }
        match self.is_blob(rev, path) {
            Ok(true) => Some(rev.len() + 1),
            Ok(false) => None,
            Err(e) => {
                eprintln!("gf: git cat-file: {e}; <rev>:path lines print in full");
                self.failed = true;
                None
            }
        }
    }

    fn is_blob(&mut self, rev: &[u8], path: &[u8]) -> io::Result<bool> {
        if self
            .pipe
            .is_none()
        {
            let mut child = Command::new("git")
                .args(["cat-file", "--batch-check=%(objecttype)"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            let stdin = child
                .stdin
                .take()
                .expect("piped");
            let stdout = child
                .stdout
                .take()
                .expect("piped");
            self.pipe = Some((stdin, BufReader::new(stdout)));
        }
        let (stdin, stdout) = self
            .pipe
            .as_mut()
            .expect("set above");

        // git grep prints paths relative to the working directory, and only a
        // leading ./ or ../ makes `<rev>:` resolve them from there.
        let mut query = rev.to_vec();
        query.push(b':');
        if !path.starts_with(b"./") && !path.starts_with(b"../") {
            query.extend_from_slice(b"./");
        }
        query.extend_from_slice(path);
        query.push(b'\n');
        stdin.write_all(&query)?;
        stdin.flush()?;

        let mut answer = Vec::new();
        if stdout.read_until(b'\n', &mut answer)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "exited before answering",
            ));
        }
        Ok(answer == b"blob\n")
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

/// Rejects a candidate that reads as code rather than a filename, before any
/// stat: a slash makes it a path outright, otherwise the punctuation that never
/// appears in a bare filename but opens most source lines rules it out.
fn is_path_shaped(cand: &[u8]) -> bool {
    if cand.is_empty() {
        return false;
    }
    if cand.contains(&b'/') {
        return true;
    }
    !cand
        .iter()
        .any(|b| {
            matches!(
                b,
                b' ' | b'\t' | b'(' | b')' | b'"' | b'\'' | b'=' | b',' | b';' | b'*'
            )
        })
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
    git: bool,
    cmd: OsString,
    stdin: bool,
    child_args: Vec<OsString>,
}

fn parse_args() -> Result<Option<Opts>, String> {
    let mut strips: Vec<Vec<u8>> = Vec::new();
    let mut show_base = true;
    let mut git = false;
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
            "--git" => git = true,
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
        git,
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
    folder: &mut Folder<impl FnMut(&[u8]) -> Option<usize>>,
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

    let mut cat_file = opts
        .git
        .then(CatFile::default);
    let mut folder = Folder::new(opts.strips, opts.show_base, |p: &[u8]| {
        find(p, cat_file.as_mut())
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
            .stderr(Stdio::piped())
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

    // Its own thread: the two pipes have independent buffers, so draining one
    // from the loop that drains the other deadlocks as soon as either fills.
    let errors = child
        .as_mut()
        .and_then(|c| {
            c.stderr
                .take()
        })
        .map(|stderr| {
            std::thread::spawn(move || {
                let mut errors = Errors::default();
                for line in BufReader::new(stderr).lines() {
                    match line {
                        Ok(line) => errors.line(&line),
                        Err(e) => {
                            eprintln!("gf: reading stderr: {e}");
                            break;
                        }
                    }
                }
                errors
            })
        });

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

    if let Some(handle) = errors {
        match handle.join() {
            Ok(errors) => {
                if let Err(e) = errors.report(&mut io::stderr().lock()) {
                    eprintln!("gf: writing the error summary: {e}");
                }
            }
            Err(_) => eprintln!("gf: the stderr reader panicked; its summary is lost"),
        }
    }

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
            move |p: &[u8]| {
                known
                    .contains(p)
                    .then_some(0)
            },
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
        "/mnt/GIT/proj/rust/test-data/deploy-configs/localhost/noans-worker-lab/config.yaml";

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
             proj/rust/test-data/deploy-configs/localhost/noans-worker-lab/config.yaml:44:  notify_command:\n\
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
    fn dir_that_prefixes_a_dashed_path_is_not_taken_as_the_path() {
        let out = fold(
            &["/g/fs-esl/types/src/a.rs", "/g/fs-esl/types/src/b.rs"],
            &["/g"],
            &[
                "/g/fs",
                "/g/fs-esl/types/src/a.rs",
                "/g/fs-esl/types/src/b.rs",
            ],
        );
        assert_eq!(
            out,
            "base: /g/\nfs-esl/types/src/a.rs\nfs-esl/types/src/b.rs\n"
        );
    }

    #[test]
    fn dashed_line_number_in_a_filename_does_not_shorten_the_path() {
        let out = fold(
            &["/a/v-1-x.rs:7:hit", "/a/v-1-x.rs-8-next"],
            &["/a"],
            &["/a/v", "/a/v-1-x.rs"],
        );
        assert_eq!(out, "base: /a/\nv-1-x.rs:7:hit\n-8-next\n");
    }

    #[test]
    fn code_leading_a_line_is_not_taken_for_a_path() {
        // `n` and `switch_stristr` both name files in the fake tree, so only the
        // shape test keeps the declaration intact.
        let out = fold(
            &["/a/u.c:1671:SWITCH_DECLARE(const char *) switch_stristr(const char *instr)"],
            &["/a"],
            &["/a/u.c", "n", "switch_stristr"],
        );
        assert_eq!(
            out,
            "base: /a/\nu.c:1671:SWITCH_DECLARE(const char *) switch_stristr(const char *instr)\n"
        );
    }

    #[test]
    fn slash_free_top_level_names_still_fold() {
        let out = fold(
            &[
                "Makefile:9:all:",
                "Makefile-10-\tcargo build",
                "Cargo.toml:6:license",
            ],
            &[],
            &["Makefile", "Cargo.toml"],
        );
        assert_eq!(
            out,
            "Makefile:9:all:\n-10-\tcargo build\nCargo.toml:6:license\n"
        );
    }

    #[test]
    fn shape_test_rejects_code_accepts_filenames() {
        for good in [
            "/a/b.c",
            "src/main.rs",
            "./LICENSE",
            "Makefile",
            "Cargo.toml",
        ] {
            assert!(is_path_shaped(good.as_bytes()), "{good}");
        }
        for bad in [
            "",
            "SWITCH_DECLARE(const char *) switch_stristr",
            "let x = foo(a, b)",
            "see Makefile",
        ] {
            assert!(!is_path_shaped(bad.as_bytes()), "{bad}");
        }
    }

    fn summarize(lines: &[&str]) -> String {
        let mut errors = Errors::default();
        for line in lines {
            errors.line(line);
        }
        let mut out = Vec::new();
        errors
            .report(&mut out)
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn every_searcher_spelling_is_counted() {
        for line in [
            "rg: /var/log/x.log: Permission denied (os error 13)",
            "ugrep: warning: cannot read /var/log/x.log: Permission denied",
            "grep: /var/log/x.log: Permission denied",
        ] {
            let d = parse_diagnostic(line).unwrap_or_else(|| panic!("{line}"));
            assert_eq!(d.kind, "Permission denied", "{line}");
            assert_eq!(d.path, "/var/log/x.log", "{line}");
        }
    }

    #[test]
    fn repeats_fold_to_a_count_and_singles_keep_their_path() {
        let out = summarize(&[
            "rg: /var/log/a.log: Permission denied (os error 13)",
            "rg: /var/log/b.log: Permission denied (os error 13)",
            "rg: /var/log/c.log: Permission denied (os error 13)",
            "rg: /nope: No such file or directory (os error 2)",
        ]);
        assert_eq!(
            out,
            "gf: 3 × Permission denied (e.g. /var/log/a.log)\n\
             gf: No such file or directory: /nope\n"
        );
    }

    /// Anything that is not a diagnostic is the searcher telling us something we
    /// have no pattern for, so it survives verbatim rather than being counted.
    #[test]
    fn other_stderr_passes_through() {
        let out = summarize(&["rg: error parsing flag --nope", "some unstructured warning"]);
        assert_eq!(
            out,
            "rg: error parsing flag --nope\nsome unstructured warning\n"
        );
    }

    fn fold_git(lines: &[&str], git: bool) -> String {
        let mut cat_file = git.then(CatFile::default);
        let mut folder = Folder::new(Vec::new(), true, |p: &[u8]| find(p, cat_file.as_mut()));
        let mut out = Vec::new();
        for line in lines {
            folder
                .line(line.as_bytes(), &mut out)
                .unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    /// Runs the real `git cat-file` against this checkout, cargo starting tests in
    /// the package root: it needs a git checkout, not a source tarball.
    #[test]
    fn git_grep_rev_prefix_folds_under_git() {
        let lines = [
            "HEAD:Cargo.toml:4:edition = \"2021\"",
            "HEAD:Cargo.toml-5-next",
            "HEAD:src/bin/gf.rs:1:a:12:b",
            "HEAD:src:3:a tree, not a file",
            "HEAD:nope.rs:3:missing",
            "HEAD^{commit}:Cargo.toml:4:x",
        ];
        assert_eq!(
            fold_git(&lines, true),
            "base: HEAD:\n\
             Cargo.toml:4:edition = \"2021\"\n\
             -5-next\n\
             src/bin/gf.rs:1:a:12:b\n\
             HEAD:src:3:a tree, not a file\n\
             HEAD:nope.rs:3:missing\n\
             base: HEAD^{commit}:\n\
             Cargo.toml:4:x\n"
        );
        assert_eq!(fold_git(&lines, false), lines.join("\n") + "\n");
    }

    #[test]
    fn split_rev_needs_both_halves_and_a_relative_path() {
        assert_eq!(
            split_rev(b"origin/main:a/b.rs"),
            Some((&b"origin/main"[..], &b"a/b.rs"[..]))
        );
        for cand in ["nocolon", ":a.rs", "HEAD:", "HEAD:/etc/passwd"] {
            assert_eq!(split_rev(cand.as_bytes()), None, "{cand}");
        }
    }

    #[test]
    fn path_equal_to_strip_prefix_is_kept() {
        let out = fold(&["/a/b:1:x"], &["/a/b"], &["/a/b"]);
        assert_eq!(out, "/a/b:1:x\n");
    }
}

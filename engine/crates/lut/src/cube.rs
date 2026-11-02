//! The reader of `.cube` files.
//!
//! It reads the Cube LUT Specification 1.0 of Adobe (2013) and the dialect that `DaVinci
//! Resolve` writes and reads, strictly where a mistake would change the picture and leniently
//! where Resolve is:
//!
//! - **Lines** end in LF, CRLF or a lone CR (the spec allows LF only, but its own samples were
//!   saved with CR, A.1). A UTF-8 byte order mark at the start is skipped. Lines of any length
//!   are read (the spec says 250 bytes at most).
//! - **Blank lines and comments** (`#` to the end of the line) may be anywhere, before, among
//!   and after the data, and a comment may follow a keyword or a row; leading and trailing
//!   spaces and tabs are ignored, and tokens are parted by any run of them.
//! - **Numbers** are decimal, with a sign, a point, an exponent (`1e-3`, `.75`, `2.`), as Rust
//!   reads an `f32`; one that is not finite as an `f32` (`inf`, `nan`, `1e39`) is refused.
//! - **Keywords**, before the data, in any order, each at most once:
//!   - `TITLE "text"` (the quotes may be left out; an opening one needs its closing one);
//!   - `LUT_1D_SIZE n` (2 to 65536) and `LUT_3D_SIZE n` (2 to 256), either or both;
//!   - `DOMAIN_MIN r g b` and `DOMAIN_MAX r g b` (Adobe; each defaults to 0 or 1);
//!   - `LUT_1D_INPUT_RANGE min max` and `LUT_3D_INPUT_RANGE min max` (Resolve).
//!
//!   A keyword that is not one of these is read past and kept in [`Lut::ignored`], as is an
//!   input range of a table that the file does not have: Resolve reads past both (a 3D file
//!   with `LUT_1D_INPUT_RANGE` is a mistake that it ignores). A keyword after the data is
//!   refused: past the first row everything must be a row.
//! - **The domain of each table.** A file with one table takes it from `DOMAIN_*` or from its
//!   `LUT_nD_INPUT_RANGE`. A file with both tables is Resolve's: the 1D table is a shaper
//!   that comes first, in the file and in the evaluation, `LUT_1D_INPUT_RANGE` is its domain
//!   and `LUT_3D_INPUT_RANGE` that of the cube; `DOMAIN_*`, which the spec gives for a file of
//!   one table, is then taken as the domain of the file's input, that of the shaper. A table
//!   that would get its domain from both `DOMAIN_*` and its input range is refused rather than
//!   guessed. Each lower bound must be below its upper one.
//! - **Rows**: three numbers each, the 1D table first (first point to last), then the cube (red
//!   changing fastest), exactly as many as the sizes say.
//!
//! Every error carries the line it was found on, from 1 (the last line, for a file that ends
//! too soon). Memory is bounded by the length of the file, whatever sizes it declares: the
//! tables grow with the rows that are there.

use crate::{Domain, Ignored, Lut, Lut1d, Lut3d, MAX_1D_SIZE, MAX_3D_SIZE, MIN_SIZE};

/// Why a file is not a `.cube`, and where.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {kind}")]
pub struct ParseError {
    /// From 1.
    pub line: usize,
    pub kind: ErrorKind,
}

/// The longest text of a file that an error keeps.
const QUOTED: usize = 40;

/// Why a file is not a `.cube`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("the line is not text (UTF-8)")]
    NotText,
    #[error("{keyword} takes {expected} values, not {found}")]
    Arity {
        keyword: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("{keyword} is given twice")]
    Repeated { keyword: &'static str },
    #[error("the keyword {keyword} comes after the rows of the table; keywords come first")]
    KeywordAfterData { keyword: String },
    #[error("the size of {keyword} is not a whole number: {text}")]
    BadSize { keyword: &'static str, text: String },
    #[error("{keyword} {size} is outside {min}..={max}")]
    SizeOutOfRange {
        keyword: &'static str,
        size: u64,
        min: usize,
        max: usize,
    },
    #[error("{text} is not a number")]
    BadNumber { text: String },
    #[error("{text} is not a finite number of 32 bits")]
    NotFinite { text: String },
    #[error("the title has no closing quote")]
    UnclosedTitle,
    #[error("the lower bound of the domain is not below the upper one")]
    DomainReversed,
    #[error("{first} and {second} both give the domain of the {table} table")]
    DomainTwice {
        first: &'static str,
        second: &'static str,
        table: &'static str,
    },
    #[error("a row of the table has {found} values, not 3")]
    RowArity { found: usize },
    #[error("there is no LUT_1D_SIZE or LUT_3D_SIZE before the rows")]
    NoSize,
    #[error("the file ends after {found} of the {expected} rows of its tables")]
    TooFewRows { expected: usize, found: usize },
    #[error("there are more rows than the {expected} of its tables")]
    TooManyRows { expected: usize },
}

/// The lines of `bytes`, each without its end (LF, CRLF or CR), numbered from 1.
struct Lines<'a> {
    rest: &'a [u8],
    number: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a [u8]);

    fn next(&mut self) -> Option<(usize, &'a [u8])> {
        if self.rest.is_empty() {
            return None;
        }
        self.number += 1;
        let end = self
            .rest
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .unwrap_or(self.rest.len());
        let line = &self.rest[..end];
        let mut next = end;
        if next < self.rest.len() {
            next += if self.rest[next] == b'\r' && self.rest.get(next + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        }
        self.rest = &self.rest[next..];
        Some((self.number, line))
    }
}

/// A keyword's values, with the line it was on.
#[derive(Default)]
struct Header {
    title: Option<String>,
    size_1d: Option<(usize, usize)>,
    size_3d: Option<(usize, usize)>,
    domain_min: Option<(usize, [f32; 3])>,
    domain_max: Option<(usize, [f32; 3])>,
    range_1d: Option<(usize, [f32; 2])>,
    range_3d: Option<(usize, [f32; 2])>,
    title_seen: bool,
}

fn quoted(text: &str) -> String {
    let mut quoted: String = text.chars().take(QUOTED).collect();
    if quoted.len() < text.len() {
        quoted.push('…');
    }
    quoted
}

fn number(token: &str) -> Result<f32, ErrorKind> {
    match token.parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(value),
        Ok(_) => Err(ErrorKind::NotFinite {
            text: quoted(token),
        }),
        Err(_) => Err(ErrorKind::BadNumber {
            text: quoted(token),
        }),
    }
}

fn numbers<const N: usize>(
    keyword: &'static str,
    (tokens, found): (&[&str], usize),
) -> Result<[f32; N], ErrorKind> {
    if found != N {
        return Err(ErrorKind::Arity {
            keyword,
            expected: N,
            found,
        });
    }
    let mut out = [0.0; N];
    for (slot, token) in out.iter_mut().zip(tokens) {
        *slot = number(token)?;
    }
    Ok(out)
}

fn size(
    keyword: &'static str,
    (tokens, found): (&[&str], usize),
    max: usize,
) -> Result<usize, ErrorKind> {
    if found != 1 {
        return Err(ErrorKind::Arity {
            keyword,
            expected: 1,
            found,
        });
    }
    let size: u64 = tokens[0].parse().map_err(|_| ErrorKind::BadSize {
        keyword,
        text: quoted(tokens[0]),
    })?;
    match usize::try_from(size) {
        Ok(n) if (MIN_SIZE..=max).contains(&n) => Ok(n),
        _ => Err(ErrorKind::SizeOutOfRange {
            keyword,
            size,
            min: MIN_SIZE,
            max,
        }),
    }
}

fn once<T>(slot: &mut Option<T>, keyword: &'static str, value: T) -> Result<(), ErrorKind> {
    if slot.is_some() {
        return Err(ErrorKind::Repeated { keyword });
    }
    *slot = Some(value);
    Ok(())
}

/// The title of a line `TITLE …`: what is between its quotes, or the rest of the line where
/// there are none.
fn title(rest: &str) -> Result<String, ErrorKind> {
    let rest = rest.trim_matches([' ', '\t']);
    match rest.strip_prefix('"') {
        Some(inner) => match inner.find('"') {
            Some(end) => Ok(inner[..end].to_owned()),
            None => Err(ErrorKind::UnclosedTitle),
        },
        None => Ok(rest.split('#').next().unwrap_or("").trim_end().to_owned()),
    }
}

impl Header {
    /// Takes in the keyword line `tokens` (its first is the keyword), or keeps an unknown one.
    fn keyword(
        &mut self,
        line: usize,
        text: &str,
        words: &Words,
        ignored: &mut Vec<Ignored>,
    ) -> Result<(), ErrorKind> {
        let values = words.values();
        match words.tokens[0] {
            "TITLE" => {
                if self.title_seen {
                    return Err(ErrorKind::Repeated { keyword: "TITLE" });
                }
                self.title_seen = true;
                let after = text.trim_start_matches([' ', '\t']);
                self.title = Some(title(&after["TITLE".len()..])?);
            }
            "LUT_1D_SIZE" => {
                let n = size("LUT_1D_SIZE", values, MAX_1D_SIZE)?;
                once(&mut self.size_1d, "LUT_1D_SIZE", (line, n))?;
            }
            "LUT_3D_SIZE" => {
                let n = size("LUT_3D_SIZE", values, MAX_3D_SIZE)?;
                once(&mut self.size_3d, "LUT_3D_SIZE", (line, n))?;
            }
            "DOMAIN_MIN" => {
                let v = numbers("DOMAIN_MIN", values)?;
                once(&mut self.domain_min, "DOMAIN_MIN", (line, v))?;
            }
            "DOMAIN_MAX" => {
                let v = numbers("DOMAIN_MAX", values)?;
                once(&mut self.domain_max, "DOMAIN_MAX", (line, v))?;
            }
            "LUT_1D_INPUT_RANGE" => {
                let v = numbers("LUT_1D_INPUT_RANGE", values)?;
                once(&mut self.range_1d, "LUT_1D_INPUT_RANGE", (line, v))?;
            }
            "LUT_3D_INPUT_RANGE" => {
                let v = numbers("LUT_3D_INPUT_RANGE", values)?;
                once(&mut self.range_3d, "LUT_3D_INPUT_RANGE", (line, v))?;
            }
            other => ignored.push(Ignored {
                line,
                keyword: other.to_owned(),
            }),
        }
        Ok(())
    }

    /// The domains of the 1D table and of the cube, from the keywords, once they are all read.
    fn domains(&self, ignored: &mut Vec<Ignored>) -> Result<(Domain, Domain), ParseError> {
        let adobe = if self.domain_min.is_some() || self.domain_max.is_some() {
            let min = self.domain_min.map_or([0.0; 3], |(_, v)| v);
            let max = self.domain_max.map_or([1.0; 3], |(_, v)| v);
            let line = self
                .domain_min
                .map_or(0, |(l, _)| l)
                .max(self.domain_max.map_or(0, |(l, _)| l));
            Some((line, Domain { min, max }))
        } else {
            None
        };
        let range = |range: Option<(usize, [f32; 2])>| {
            range.map(|(line, [min, max])| (line, Domain::uniform(min, max)))
        };
        let mut range_1d = range(self.range_1d);
        let mut range_3d = range(self.range_3d);
        if self.size_1d.is_none() {
            if let Some((line, _)) = range_1d.take() {
                ignored.push(Ignored {
                    line,
                    keyword: "LUT_1D_INPUT_RANGE".into(),
                });
            }
        }
        if self.size_3d.is_none() {
            if let Some((line, _)) = range_3d.take() {
                ignored.push(Ignored {
                    line,
                    keyword: "LUT_3D_INPUT_RANGE".into(),
                });
            }
        }
        ignored.sort_by_key(|i| i.line);
        // DOMAIN_* is the domain of the file's input: of its first table.
        let (adobe_1d, adobe_3d) = if self.size_1d.is_some() {
            (adobe, None)
        } else {
            (None, adobe)
        };
        let pick = |adobe: Option<(usize, Domain)>,
                    range: Option<(usize, Domain)>,
                    keyword: &'static str,
                    table: &'static str|
         -> Result<Domain, ParseError> {
            match (adobe, range) {
                (Some((first, _)), Some((second, _))) => Err(ParseError {
                    line: first.max(second),
                    kind: ErrorKind::DomainTwice {
                        first: "DOMAIN_MIN/DOMAIN_MAX",
                        second: keyword,
                        table,
                    },
                }),
                (Some((line, domain)), None) | (None, Some((line, domain))) => {
                    if domain.is_valid() {
                        Ok(domain)
                    } else {
                        Err(ParseError {
                            line,
                            kind: ErrorKind::DomainReversed,
                        })
                    }
                }
                (None, None) => Ok(Domain::default()),
            }
        };
        Ok((
            pick(adobe_1d, range_1d, "LUT_1D_INPUT_RANGE", "1D")?,
            pick(adobe_3d, range_3d, "LUT_3D_INPUT_RANGE", "3D")?,
        ))
    }
}

/// The rows that are read, into a table of `expected` rows that grows only with what is there.
struct Rows {
    expected_1d: usize,
    expected: usize,
    domains: (Domain, Domain),
    table: Vec<[f32; 3]>,
}

impl Rows {
    /// The rows of the tables of `header`, at the first row, `rest` bytes before the end.
    fn start(
        header: &Header,
        ignored: &mut Vec<Ignored>,
        line: usize,
        rest: usize,
    ) -> Result<Rows, ParseError> {
        let expected_1d = header.size_1d.map_or(0, |(_, n)| n);
        let expected_3d = header.size_3d.map_or(0, |(_, n)| n * n * n);
        if expected_1d + expected_3d == 0 {
            return Err(ParseError {
                line,
                kind: ErrorKind::NoSize,
            });
        }
        let domains = header.domains(ignored)?;
        let expected = expected_1d + expected_3d;
        // No more than the rows that the rest of the file can hold, at 6 bytes ("0 0 0" and
        // its end) the shortest.
        let room = rest / 6 + 1;
        Ok(Rows {
            expected_1d,
            expected,
            domains,
            table: Vec::with_capacity(expected.min(room)),
        })
    }

    fn push(&mut self, row: [f32; 3]) -> Result<(), ErrorKind> {
        if self.table.len() == self.expected {
            return Err(ErrorKind::TooManyRows {
                expected: self.expected,
            });
        }
        self.table.push(row);
        Ok(())
    }

    /// The LUT, at the end of the file, on line `last`.
    fn finish(
        mut self,
        header: Header,
        ignored: Vec<Ignored>,
        last: usize,
    ) -> Result<Lut, ParseError> {
        if self.table.len() < self.expected {
            return Err(ParseError {
                line: last,
                kind: ErrorKind::TooFewRows {
                    expected: self.expected,
                    found: self.table.len(),
                },
            });
        }
        // Every check of a table was made as it was read: its size, its domain, its values.
        let cube_rows = self.table.split_off(self.expected_1d);
        let shaper = (self.expected_1d > 0).then_some(Lut1d {
            domain: self.domains.0,
            table: self.table,
        });
        let cube = header.size_3d.map(|(_, size)| Lut3d {
            domain: self.domains.1,
            size,
            table: cube_rows,
        });
        Ok(Lut {
            title: header.title,
            shaper,
            cube,
            ignored,
        })
    }
}

/// The words of a line without its comment, if it has any: none for a blank line or a comment.
fn words(text: &str) -> Words<'_> {
    // The comment at its end, but for a title, whose quotes may hold a `#`.
    let first = text.split_ascii_whitespace().next().unwrap_or("");
    let body = if first == "TITLE" {
        text
    } else {
        text.split('#').next().unwrap_or("")
    };
    Words::of(body)
}

/// Reads a `.cube` file.
///
/// # Errors
/// It is not one: see [`ErrorKind`] and the rules of [`crate::cube`].
pub fn parse_cube(bytes: &[u8]) -> Result<Lut, ParseError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut header = Header::default();
    let mut ignored = Vec::new();
    let mut rows: Option<Rows> = None;
    let mut last_line = 0;
    let mut lines = Lines {
        rest: bytes,
        number: 0,
    };
    while let Some((line, raw)) = lines.next() {
        last_line = line;
        let at = |kind| ParseError { line, kind };
        if raw
            .iter()
            .find(|b| **b != b' ' && **b != b'\t')
            .is_none_or(|b| *b == b'#')
        {
            // Blank, or a comment, which need not be text.
            continue;
        }
        let text = std::str::from_utf8(raw).map_err(|_| at(ErrorKind::NotText))?;
        let words = words(text);
        let Some(first) = words.tokens.first() else {
            continue;
        };
        if first.parse::<f32>().is_err() {
            if rows.is_some() {
                return Err(at(ErrorKind::KeywordAfterData {
                    keyword: quoted(first),
                }));
            }
            header
                .keyword(line, text, &words, &mut ignored)
                .map_err(at)?;
            continue;
        }
        let table = if let Some(table) = rows.as_mut() {
            table
        } else {
            rows.insert(Rows::start(&header, &mut ignored, line, lines.rest.len())?)
        };
        table.push(row(&words).map_err(at)?).map_err(at)?;
    }
    if let Some(rows) = rows {
        return rows.finish(header, ignored, last_line);
    }
    let kind = if header.size_1d.is_some() || header.size_3d.is_some() {
        let expected =
            header.size_1d.map_or(0, |(_, n)| n) + header.size_3d.map_or(0, |(_, n)| n * n * n);
        ErrorKind::TooFewRows { expected, found: 0 }
    } else {
        ErrorKind::NoSize
    };
    Err(ParseError {
        line: last_line.max(1),
        kind,
    })
}

/// The words of a line: the first few, and how many there are.
struct Words<'a> {
    tokens: Vec<&'a str>,
    count: usize,
}

impl<'a> Words<'a> {
    /// No line of a `.cube` has more than 4 words; a longer one is kept to its first 5 (and
    /// its count), so that a line of a million words is not a vector of them.
    fn of(body: &'a str) -> Words<'a> {
        let tokens: Vec<&str> = body.split_ascii_whitespace().take(5).collect();
        let count = if tokens.len() < 5 {
            tokens.len()
        } else {
            body.split_ascii_whitespace().count()
        };
        Words { tokens, count }
    }

    /// The words after the first, and how many there are.
    fn values(&self) -> (&[&'a str], usize) {
        (&self.tokens[1..], self.count - 1)
    }
}

fn row(words: &Words) -> Result<[f32; 3], ErrorKind> {
    if words.count != 3 {
        return Err(ErrorKind::RowArity { found: words.count });
    }
    let t = &words.tokens;
    Ok([number(t[0])?, number(t[1])?, number(t[2])?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpolation;

    fn parse(text: &str) -> Result<Lut, ParseError> {
        parse_cube(text.as_bytes())
    }

    fn error(text: &str) -> (usize, ErrorKind) {
        let e = parse(text).expect_err("an error");
        (e.line, e.kind)
    }

    const ADOBE_3D: &str = "# Created Tue 19 Mar 2013\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 .75 0\n\
                            1 .75 0\n0 .25 1\n1 .25 1\n0 1 1\n1 1 1\n";

    #[test]
    fn the_sample_cube_of_the_spec_is_its_function() {
        // A.4: {r, (3g + b) / 4, b}, red changing fastest.
        let lut = parse(ADOBE_3D).unwrap();
        let cube = lut.cube().unwrap();
        assert_eq!(cube.size(), 2);
        assert_eq!(cube.at(1, 0, 0), [1.0, 0.0, 0.0]);
        assert_eq!(cube.at(0, 1, 0), [0.0, 0.75, 0.0]);
        assert_eq!(cube.at(0, 0, 1), [0.0, 0.25, 1.0]);
        assert!(lut.shaper().is_none());
        assert_eq!(lut.title(), None);
        for interpolation in [Interpolation::Tetrahedral, Interpolation::Trilinear] {
            let [r, g, b] = lut.eval([0.3, 0.6, 0.2], interpolation);
            assert!((r - 0.3).abs() < 1e-6 && (g - 0.5).abs() < 1e-6 && (b - 0.2).abs() < 1e-6);
        }
    }

    #[test]
    fn the_sample_of_mixed_domains_clamps_beyond_its_points() {
        // A.3: y = x for x <= 1, each channel of its own domain.
        let lut = parse(
            "TITLE \"Demo\"\nLUT_1D_SIZE 3\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 2 3\n0 0 0\n\
             # Comments can go anywhere\n0.5 1 1.5\n1 1 1\n",
        )
        .unwrap();
        assert_eq!(lut.title(), Some("Demo"));
        let shaper = lut.shaper().unwrap();
        assert_eq!(shaper.domain().max, [1.0, 2.0, 3.0]);
        let [r, g, b] = lut.eval([0.25, 0.5, 0.75], Interpolation::default());
        // Each channel is the identity over 0..1 of its own domain.
        assert!((r - 0.25).abs() < 1e-6 && (g - 0.5).abs() < 1e-6 && (b - 0.75).abs() < 1e-6);
        assert_eq!(
            lut.eval([5.0, 5.0, -1.0], Interpolation::default()),
            [1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn resolve_files_of_a_shaper_and_a_cube_read_the_shaper_first() {
        let text = "TITLE \"Shaped\"\r\nLUT_1D_SIZE 2\r\nLUT_1D_INPUT_RANGE -0.5 2.5\r\n\
                    LUT_3D_SIZE 2\r\nLUT_3D_INPUT_RANGE 0 1\r\n0 0 0\r\n1 1 1\r\n\
                    0 0 0\r\n1 0 0\r\n0 1 0\r\n1 1 0\r\n0 0 1\r\n1 0 1\r\n0 1 1\r\n1 1 1\r\n";
        let lut = parse(text).unwrap();
        assert_eq!(lut.shaper().unwrap().domain(), Domain::uniform(-0.5, 2.5));
        assert_eq!(lut.cube().unwrap().domain(), Domain::default());
        assert_eq!(lut.cube().unwrap().at(1, 1, 0), [1.0, 1.0, 0.0]);
        // -0.5..2.5 to 0..1, then the identity.
        let [r, g, b] = lut.eval([1.0, -0.5, 2.5], Interpolation::default());
        assert!((r - 0.5).abs() < 1e-6 && g.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lines_of_every_ending_comments_blanks_spaces_and_exponents_are_read() {
        let text = "\u{feff}  # a comment\r\rTITLE  \"a # in a title\"  \r\
                    \tLUT_1D_SIZE\t 2 # the size\n\n  0.0e0   -1E-1\t+2.5e+0  \n\
                    # between\r\n1. .5 3   # after a row\n\n# and at the end";
        let lut = parse(text).unwrap();
        assert_eq!(lut.title(), Some("a # in a title"));
        assert_eq!(
            lut.shaper().unwrap().table(),
            [[0.0, -0.1, 2.5], [1.0, 0.5, 3.0]]
        );
        assert_eq!(lut.ignored(), []);
    }

    #[test]
    fn a_title_without_quotes_is_the_rest_of_its_line() {
        let lut = parse("TITLE film look # v2\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        assert_eq!(lut.title(), Some("film look"));
        let lut = parse("TITLE \"\"\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        assert_eq!(lut.title(), Some(""));
    }

    #[test]
    fn keywords_that_resolve_reads_past_are_kept() {
        let lut = parse(
            "LUT_1D_INPUT_RANGE 0 1\nLUT_IN_VIDEO_RANGE\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n\
             1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        )
        .unwrap();
        let ignored: Vec<_> = lut
            .ignored()
            .iter()
            .map(|i| (i.line, i.keyword.as_str()))
            .collect();
        assert_eq!(
            ignored,
            [(1, "LUT_1D_INPUT_RANGE"), (2, "LUT_IN_VIDEO_RANGE")]
        );
        let lut = parse("LUT_3D_INPUT_RANGE 0 2\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        assert_eq!(lut.ignored()[0].keyword, "LUT_3D_INPUT_RANGE");
        assert_eq!(lut.shaper().unwrap().domain(), Domain::default());
    }

    #[test]
    fn domain_min_and_max_are_of_the_first_table() {
        let lut = parse(
            "LUT_1D_SIZE 2\nLUT_3D_SIZE 2\nDOMAIN_MAX 4 4 4\nLUT_3D_INPUT_RANGE 0 2\n\
             0 0 0\n1 1 1\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        )
        .unwrap();
        assert_eq!(lut.shaper().unwrap().domain(), Domain::uniform(0.0, 4.0));
        assert_eq!(lut.cube().unwrap().domain(), Domain::uniform(0.0, 2.0));
        let lut = parse("DOMAIN_MIN -1 -1 -1\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();
        assert_eq!(lut.cube().unwrap().domain(), Domain::uniform(-1.0, 1.0));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_error_says_its_line() {
        assert_eq!(
            parse_cube(b"LUT_1D_SIZE 2\n0 0 \xff\n"),
            Err(ParseError {
                line: 2,
                kind: ErrorKind::NotText
            })
        );
        // Bytes that are not text, but in a comment, are a comment.
        assert!(parse_cube(b"# \xff\xfe\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n").is_ok());
        assert_eq!(
            error("LUT_3D_SIZE 2 3\n"),
            (
                1,
                ErrorKind::Arity {
                    keyword: "LUT_3D_SIZE",
                    expected: 1,
                    found: 2
                }
            )
        );
        assert_eq!(
            error("\nDOMAIN_MIN 0 0\n"),
            (
                2,
                ErrorKind::Arity {
                    keyword: "DOMAIN_MIN",
                    expected: 3,
                    found: 2
                }
            )
        );
        assert_eq!(
            error("LUT_1D_INPUT_RANGE 0\n").1,
            ErrorKind::Arity {
                keyword: "LUT_1D_INPUT_RANGE",
                expected: 2,
                found: 1
            }
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\nLUT_1D_SIZE 2\n"),
            (
                2,
                ErrorKind::Repeated {
                    keyword: "LUT_1D_SIZE"
                }
            )
        );
        assert_eq!(
            error("TITLE \"a\"\nTITLE \"b\"\n"),
            (2, ErrorKind::Repeated { keyword: "TITLE" })
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0\nTITLE \"late\"\n1 1 1\n"),
            (
                3,
                ErrorKind::KeywordAfterData {
                    keyword: "TITLE".into()
                }
            )
        );
        assert_eq!(
            error("LUT_3D_SIZE 17.0\n"),
            (
                1,
                ErrorKind::BadSize {
                    keyword: "LUT_3D_SIZE",
                    text: "17.0".into()
                }
            )
        );
        assert_eq!(
            error("LUT_3D_SIZE -3\n").1,
            ErrorKind::BadSize {
                keyword: "LUT_3D_SIZE",
                text: "-3".into()
            }
        );
        for (text, keyword, size, max) in [
            ("LUT_3D_SIZE 257", "LUT_3D_SIZE", 257, 256),
            ("LUT_3D_SIZE 1", "LUT_3D_SIZE", 1, 256),
            ("LUT_1D_SIZE 65537", "LUT_1D_SIZE", 65537, 65536),
            ("LUT_1D_SIZE 0", "LUT_1D_SIZE", 0, 65536),
            (
                "LUT_1D_SIZE 18446744073709551615",
                "LUT_1D_SIZE",
                u64::MAX,
                65536,
            ),
        ] {
            assert_eq!(
                error(text),
                (
                    1,
                    ErrorKind::SizeOutOfRange {
                        keyword,
                        size,
                        min: 2,
                        max
                    }
                ),
                "{text}"
            );
        }
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0\n1 x 1\n"),
            (3, ErrorKind::BadNumber { text: "x".into() })
        );
        assert_eq!(
            error("DOMAIN_MAX 1 1 1,0\n").1,
            ErrorKind::BadNumber { text: "1,0".into() }
        );
        for bad in ["inf", "NaN", "1e39", "-1e39"] {
            assert_eq!(
                error(&format!("LUT_1D_SIZE 2\n0 0 0\n1 {bad} 1\n")),
                (3, ErrorKind::NotFinite { text: bad.into() }),
                "{bad}"
            );
        }
        // A row that starts with one is a row too.
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0\nnan 0 0\n").1,
            ErrorKind::NotFinite { text: "nan".into() }
        );
        assert_eq!(
            error("TITLE \"open\nLUT_1D_SIZE 2\n"),
            (1, ErrorKind::UnclosedTitle)
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\nDOMAIN_MIN 0 1 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n"),
            (3, ErrorKind::DomainReversed)
        );
        assert_eq!(
            error("LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 1 -1\n0 0 0\n"),
            (2, ErrorKind::DomainReversed)
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\nDOMAIN_MAX 2 2 2\nLUT_1D_INPUT_RANGE 0 2\n0 0 0\n1 1 1\n"),
            (
                3,
                ErrorKind::DomainTwice {
                    first: "DOMAIN_MIN/DOMAIN_MAX",
                    second: "LUT_1D_INPUT_RANGE",
                    table: "1D"
                }
            )
        );
        assert_eq!(
            error("LUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0 2\nDOMAIN_MIN 0 0 0\n0 0 0\n").1,
            ErrorKind::DomainTwice {
                first: "DOMAIN_MIN/DOMAIN_MAX",
                second: "LUT_3D_INPUT_RANGE",
                table: "3D"
            }
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0\n1 1\n"),
            (3, ErrorKind::RowArity { found: 2 })
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0 0\n").1,
            ErrorKind::RowArity { found: 4 }
        );
        assert_eq!(error("TITLE \"x\"\n0 0 0\n"), (2, ErrorKind::NoSize));
        assert_eq!(error("# nothing\n\n"), (2, ErrorKind::NoSize));
        assert_eq!(error(""), (1, ErrorKind::NoSize));
        assert_eq!(
            error("LUT_3D_SIZE 2\n0 0 0\n1 0 0\n# then nothing\n"),
            (
                4,
                ErrorKind::TooFewRows {
                    expected: 8,
                    found: 2
                }
            )
        );
        assert_eq!(
            error("LUT_1D_SIZE 3\nLUT_3D_SIZE 2\n"),
            (
                2,
                ErrorKind::TooFewRows {
                    expected: 11,
                    found: 0
                }
            )
        );
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n2 2 2\n"),
            (4, ErrorKind::TooManyRows { expected: 2 })
        );
    }

    #[test]
    fn errors_read_as_sentences_with_their_lines() {
        let e = parse("LUT_1D_SIZE 2\n0 0 0\n1 1\n").unwrap_err();
        assert_eq!(
            e.to_string(),
            "line 3: a row of the table has 2 values, not 3"
        );
        let long = "x".repeat(1000);
        let e = parse(&format!("LUT_1D_SIZE 2\n0 0 {long}\n")).unwrap_err();
        assert!(e.to_string().len() < 100, "{e}");
        // The text that an error keeps is its first 40 characters and an ellipsis.
        assert_eq!(
            e.kind,
            ErrorKind::BadNumber {
                text: format!("{}…", "x".repeat(40))
            }
        );
    }

    #[test]
    fn errors_count_every_value_of_a_long_line_and_lines_of_crlf_once() {
        // A line is kept to its first five words, but its count is of all of them.
        assert_eq!(
            error("LUT_1D_SIZE 2\n0 0 0 0 0 0 0\n"),
            (2, ErrorKind::RowArity { found: 7 })
        );
        assert_eq!(
            error("DOMAIN_MIN 0 0 0 0 0 0\n").1,
            ErrorKind::Arity {
                keyword: "DOMAIN_MIN",
                expected: 3,
                found: 6
            }
        );
        // CRLF ends a line, not two.
        assert_eq!(
            error("LUT_1D_SIZE 2\r\n0 0 0\r\n1 x 1\r\n"),
            (3, ErrorKind::BadNumber { text: "x".into() })
        );
        // A file of one table and no rows is short of all of them.
        assert_eq!(
            error("LUT_3D_SIZE 2\n"),
            (
                1,
                ErrorKind::TooFewRows {
                    expected: 8,
                    found: 0
                }
            )
        );
        assert_eq!(
            error("# a\nLUT_1D_SIZE 4\n").1,
            ErrorKind::TooFewRows {
                expected: 4,
                found: 0
            }
        );
        // A comment after spaces and tabs need not be text either.
        assert!(parse_cube(b" \t # \xff\xfe\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n").is_ok());
    }

    #[test]
    fn what_is_written_reads_back_as_it_was() {
        for text in [
            ADOBE_3D,
            "TITLE \"Demo\"\nLUT_1D_SIZE 3\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 2 3\n0 0 0\n0.5 1 1.5\n1 1 1\n",
            "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE -0.25 4\nLUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0 2\n\
             0.1 0.2 0.30000001\n1 1 1\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1e-7\n",
            "LUT_1D_SIZE 2\nDOMAIN_MAX 1 2 3\nLUT_3D_SIZE 2\n\
             0 0 0\n1 1 1\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        ] {
            let lut = parse(text).unwrap();
            let written = lut.to_cube().unwrap();
            assert_eq!(parse(&written).unwrap(), lut, "{written}");
        }
    }
}

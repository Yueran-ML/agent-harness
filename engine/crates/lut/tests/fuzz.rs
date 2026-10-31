//! The reader on files that are mangled at random: it answers every one with a LUT or an
//! error, never a panic, and what it reads is a LUT through and through. And every LUT that is
//! written reads back as itself.

use columbina_lut::{parse_cube, Domain, Interpolation, Lut, Lut1d, Lut3d, ParseError};
use proptest::prelude::*;

/// A file that reads: a shaper, a cube, or both, with some of the dressing that files carry.
fn good_file() -> impl Strategy<Value = String> {
    (
        0_usize..3,
        2_usize..6,
        2_usize..5,
        prop::collection::vec(-1.0e3_f32..1.0e3, 1..40),
        any::<bool>(),
    )
        .prop_map(|(shape, n1, n3, values, crlf)| {
            let mut lines = vec![
                "# made for a test".to_owned(),
                "TITLE \"fuzz # title\"".to_owned(),
            ];
            let (has_1d, has_3d) = [(true, false), (false, true), (true, true)][shape];
            if has_1d {
                lines.push(format!("LUT_1D_SIZE {n1}"));
                lines.push("LUT_1D_INPUT_RANGE -0.5 1.5".into());
            }
            if has_3d {
                lines.push(format!("LUT_3D_SIZE {n3}"));
                if !has_1d {
                    lines.push("DOMAIN_MIN 0 0 0".into());
                    lines.push("DOMAIN_MAX 1 2 1".into());
                }
            }
            let rows = if has_1d { n1 } else { 0 } + if has_3d { n3 * n3 * n3 } else { 0 };
            for i in 0..rows {
                let v = |k: usize| values[(i * 3 + k) % values.len()];
                lines.push(format!("{} {:e} {}", v(0), v(1), v(2)));
            }
            lines.join(if crlf { "\r\n" } else { "\n" })
        })
}

#[derive(Clone, Debug)]
enum Mutation {
    Delete(usize),
    Duplicate(usize),
    Swap(usize, usize),
    Insert(usize, String),
    ReplaceToken(usize, usize, String),
    Truncate(usize),
    FlipByte(usize, u8),
}

fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("#".to_owned()),
        Just("\"".to_owned()),
        Just("nan".to_owned()),
        Just("-inf".to_owned()),
        Just("1e39".to_owned()),
        Just("LUT_3D_SIZE".to_owned()),
        Just("LUT_1D_SIZE".to_owned()),
        Just("256".to_owned()),
        Just("65536".to_owned()),
        Just("4294967296".to_owned()),
        Just("18446744073709551616".to_owned()),
        Just("DOMAIN_MIN".to_owned()),
        Just("TITLE".to_owned()),
        Just("\u{feff}".to_owned()),
        Just("\r".to_owned()),
        "[ -~]{0,12}",
        any::<f32>().prop_map(|f| f.to_string()),
    ]
}

fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("LUT_3D_SIZE 256".to_owned()),
        Just("LUT_1D_SIZE 65536".to_owned()),
        Just("LUT_3D_SIZE 2".to_owned()),
        Just("LUT_3D_INPUT_RANGE 1 0".to_owned()),
        Just("DOMAIN_MAX 0 0 0".to_owned()),
        Just("0 0 0".to_owned()),
        Just("TITLE \"".to_owned()),
        prop::collection::vec(token(), 0..6).prop_map(|t| t.join(" ")),
    ]
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        any::<usize>().prop_map(Mutation::Delete),
        any::<usize>().prop_map(Mutation::Duplicate),
        (any::<usize>(), any::<usize>()).prop_map(|(a, b)| Mutation::Swap(a, b)),
        (any::<usize>(), line()).prop_map(|(a, l)| Mutation::Insert(a, l)),
        (any::<usize>(), any::<usize>(), token())
            .prop_map(|(a, b, t)| Mutation::ReplaceToken(a, b, t)),
        any::<usize>().prop_map(Mutation::Truncate),
        (any::<usize>(), any::<u8>()).prop_map(|(a, b)| Mutation::FlipByte(a, b)),
    ]
}

fn mangle(file: &str, mutations: &[Mutation]) -> Vec<u8> {
    let mut lines: Vec<String> = file.split('\n').map(str::to_owned).collect();
    let mut tail: Vec<(usize, u8)> = Vec::new();
    let mut cut = None;
    for m in mutations {
        let n = lines.len().max(1);
        match m {
            Mutation::Delete(at) if !lines.is_empty() => {
                lines.remove(at % n);
            }
            Mutation::Duplicate(at) if !lines.is_empty() => {
                let copy = lines[at % n].clone();
                lines.insert(at % n, copy);
            }
            Mutation::Swap(a, b) if !lines.is_empty() => lines.swap(a % n, b % n),
            Mutation::Insert(at, line) => lines.insert(at % (lines.len() + 1), line.clone()),
            Mutation::ReplaceToken(at, which, token) if !lines.is_empty() => {
                let mut words: Vec<String> = lines[at % n].split(' ').map(str::to_owned).collect();
                let k = which % words.len();
                words[k].clone_from(token);
                lines[at % n] = words.join(" ");
            }
            Mutation::Truncate(at) => cut = Some(*at),
            Mutation::FlipByte(at, byte) => tail.push((*at, *byte)),
            _ => {}
        }
    }
    let mut bytes = lines.join("\n").into_bytes();
    for (at, byte) in tail {
        if !bytes.is_empty() {
            let i = at % bytes.len();
            bytes[i] = byte;
        }
    }
    if let Some(at) = cut {
        bytes.truncate(at % (bytes.len() + 1));
    }
    bytes
}

/// What is read is whole: tables of their sizes, of finite values, with domains that are ones.
fn check(lut: &Lut) {
    assert!(lut.shaper().is_some() || lut.cube().is_some());
    if let Some(shaper) = lut.shaper() {
        assert!((2..=65536).contains(&shaper.size()));
        assert!(shaper.domain().is_valid());
        assert!(shaper.table().iter().flatten().all(|v| v.is_finite()));
    }
    if let Some(cube) = lut.cube() {
        assert!((2..=256).contains(&cube.size()));
        assert_eq!(cube.table().len(), cube.size().pow(3));
        assert!(cube.domain().is_valid());
        assert!(cube.table().iter().flatten().all(|v| v.is_finite()));
    }
    for rgb in [[0.0; 3], [0.5, 1.5, -0.5], [1.0; 3]] {
        for interpolation in [Interpolation::Tetrahedral, Interpolation::Trilinear] {
            let _ = lut.eval(rgb, interpolation);
        }
    }
    // And it writes and reads back as itself, where a .cube can say it.
    if let Ok(text) = lut.to_cube() {
        let again = parse_cube(text.as_bytes()).expect("what is written reads");
        assert_eq!(again.shaper(), lut.shaper());
        assert_eq!(again.cube(), lut.cube());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn good_files_read(file in good_file()) {
        let lut = parse_cube(file.as_bytes()).unwrap();
        check(&lut);
    }

    #[test]
    fn mangled_files_are_read_or_refused_and_never_panic(
        file in good_file(),
        mutations in prop::collection::vec(mutation(), 1..6),
    ) {
        let bytes = mangle(&file, &mutations);
        match parse_cube(&bytes) {
            Ok(lut) => check(&lut),
            Err(ParseError { line, .. }) => {
                let lines = bytes.iter().filter(|b| **b == b'\n' || **b == b'\r').count() + 1;
                prop_assert!(line >= 1 && line <= lines, "line {line} of {lines}");
            }
        }
    }

    #[test]
    fn random_bytes_are_read_or_refused(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(lut) = parse_cube(&bytes) {
            check(&lut);
        }
    }

    #[test]
    fn luts_made_in_code_are_written_and_read_back_exactly(
        n1 in 2_usize..20,
        n3 in 2_usize..6,
        values in prop::collection::vec(any::<f32>().prop_filter("finite", |v| v.is_finite()), 3..30),
        min in -10.0_f32..0.0,
        width in 0.001_f32..10.0,
        shape in 0_usize..3,
    ) {
        let value = |i: usize, c: usize| values[(i * 3 + c) % values.len()];
        let domain = Domain::uniform(min, min + width);
        let shaper = Lut1d::new(domain, (0..n1).map(|i| [0, 1, 2].map(|c| value(i, c))).collect()).unwrap();
        let cube = Lut3d::new(n3, domain, (0..n3.pow(3)).map(|i| [0, 1, 2].map(|c| value(i + 7, c))).collect()).unwrap();
        let (shaper, cube) = match shape {
            0 => (Some(shaper), None),
            1 => (None, Some(cube)),
            _ => (Some(shaper), Some(cube)),
        };
        let lut = Lut::new(Some("a \"quoted\" title".into()), shaper, cube).unwrap();
        let read = parse_cube(lut.to_cube().unwrap().as_bytes()).unwrap();
        prop_assert_eq!(read.shaper(), lut.shaper());
        prop_assert_eq!(read.cube(), lut.cube());
        prop_assert_eq!(read.title(), Some("a 'quoted' title"));
    }
}

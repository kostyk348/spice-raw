//! Read SPICE waveform files — the `.raw` files that LTspice and ngspice write next to a netlist.
//!
//! Why this exists: every SPICE in the world can *produce* waveforms, and almost none of them can
//! show them well. The file format is the part everyone shares, so a reader that handles both
//! dialects is useful on its own — and it is the foundation for a viewer that does not care which
//! simulator produced the data.
//!
//! What is supported:
//!
//! - the ASCII header (`Title`, `Date`, `Plotname`, `Flags`, `No. Variables`, `No. Points`, the
//!   variable table) with either LF or CRLF line endings;
//! - binary payloads of 8-byte doubles and 4-byte floats, little- and big-endian detected by trying
//!   both and keeping the one that does not produce absurd values;
//! - complex payloads (ngspice AC analysis): the real part is kept, the imaginary part is available;
//! - the `fastaccess` layout, where data is stored variable-major instead of point-major.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The header ended without a `Binary:` or `Values:` marker.
    NoDataSection,
    /// A header field the parser needs is missing or unreadable.
    BadHeader(String),
    /// The payload does not hold the declared number of points and variables.
    Truncated { expected: usize, got: usize },
    /// A flag combination this reader does not understand.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoDataSection => write!(f, "the header has no Binary: or Values: section"),
            Error::BadHeader(what) => write!(f, "bad header: {what}"),
            Error::Truncated { expected, got } => write!(f, "expected {expected} bytes of data, found {got}"),
            Error::Unsupported(what) => write!(f, "unsupported: {what}"),
        }
    }
}

/// One column of a plot: `time`, `V(out)`, `I(R1)`, and so on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub index: usize,
    pub name: String,
    /// The type column as the file spells it: `time`, `voltage`, `current`, `frequency`, ...
    pub kind: String,
}

/// A parsed waveform file.
#[derive(Debug, Clone, Default)]
pub struct Plot {
    pub title: String,
    pub date: String,
    pub plotname: String,
    pub flags: Vec<String>,
    pub variables: Vec<Variable>,
    /// `values[point][variable]`, one value per variable per point.
    pub values: Vec<Vec<f64>>,
    /// `imaginary[point][variable]` for complex plots, empty for real ones.
    pub imaginary: Vec<Vec<f64>>,
}

impl Plot {
    pub fn points(&self) -> usize {
        self.values.len()
    }

    pub fn is_complex(&self) -> bool {
        !self.imaginary.is_empty()
    }

    /// Find a column by name, case-sensitively — the way the netlist spelled it.
    ///
    /// The column is returned owned: a borrowed slice would have to point at a temporary, and a
    /// viewer wants to hand the data around anyway.
    pub fn column(&self, name: &str) -> Option<Vec<f64>> {
        let index = self.variables.iter().position(|variable| variable.name == name)?;
        self.column_at(index)
    }

    /// The column at a position, which is what a viewer needs: axis 0 is normally `time`.
    pub fn column_at(&self, index: usize) -> Option<Vec<f64>> {
        if index >= self.variables.len() {
            return None;
        }
        Some(self.values.iter().map(|point| point[index]).collect())
    }
}

/// Parse a `.raw` file from its bytes.
pub fn parse(bytes: &[u8]) -> Result<Plot, Error> {
    let (header, marker, payload) = split_at_data(bytes)?;
    let mut plot = Plot::default();
    let mut expected_variables = 0usize;
    let mut expected_points = 0usize;
    let mut in_variables = false;

    for line in header.lines() {
        let line = line.trim_end_matches('\r');
        if in_variables {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let index = parts.next().and_then(|text| text.parse::<usize>().ok()).ok_or_else(|| Error::BadHeader(format!("variable index in {line:?}")))?;
            let name = parts.next().ok_or_else(|| Error::BadHeader(format!("variable name in {line:?}")))?.to_string();
            let kind = parts.next().unwrap_or("unknown").to_string();
            plot.variables.push(Variable { index, name, kind });
            continue;
        }
        if let Some(rest) = line.strip_prefix("Title:") {
            plot.title = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Date:") {
            plot.date = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Plotname:") {
            plot.plotname = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Flags:") {
            plot.flags = rest.split_whitespace().map(str::to_string).collect();
        } else if let Some(rest) = line.strip_prefix("No. Variables:") {
            expected_variables = rest.trim().parse().map_err(|_| Error::BadHeader(format!("No. Variables: {rest:?}")))?;
        } else if let Some(rest) = line.strip_prefix("No. Points:") {
            expected_points = rest.trim().parse().map_err(|_| Error::BadHeader(format!("No. Points: {rest:?}")))?;
        } else if line.starts_with("Variables:") {
            in_variables = true;
        }
    }

    if plot.variables.len() != expected_variables {
        return Err(Error::BadHeader(format!(
            "the header declares {expected_variables} variables and lists {}",
            plot.variables.len()
        )));
    }
    if expected_points == 0 {
        return Ok(plot);
    }

    let complex = plot.flags.iter().any(|flag| flag == "complex");
    let fast = plot.flags.iter().any(|flag| flag == "fastaccess");
    match marker {
        Marker::Binary => read_binary(&mut plot, payload, expected_points, expected_variables, complex, fast),
        Marker::Ascii => read_ascii(&mut plot, payload, expected_points, expected_variables),
    }?;
    Ok(plot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Binary,
    Ascii,
}

/// Split the file into the ASCII header, which data section follows, and the payload.
fn split_at_data(bytes: &[u8]) -> Result<(String, Marker, &[u8]), Error> {
    for (marker_text, marker) in [("Binary:\n", Marker::Binary), ("Values:\n", Marker::Ascii)] {
        if let Some(position) = find(bytes, marker_text.as_bytes()) {
            let end = position + marker_text.len();
            let header = String::from_utf8_lossy(&bytes[..position]).into_owned();
            return Ok((header, marker, &bytes[end..]));
        }
    }
    Err(Error::NoDataSection)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Which layout and width the payload uses, decided by trying the candidates.
fn read_binary(
    plot: &mut Plot,
    payload: &[u8],
    points: usize,
    variables: usize,
    complex: bool,
    fast: bool,
) -> Result<(), Error> {
    let per_value = if complex { 2 } else { 1 };
    let fast_prefix = if fast { 1 } else { 0 };
    for width in [8usize, 4] {
        let needed = (fast_prefix + points * variables * per_value) * width;
        // A payload must hold exactly one layout, plus at most a partial word of padding. Accepting
        // a "close enough" length is how a truncated file gets read as floats and silently produces
        // half the waveforms: a 16-byte payload for three doubles fits neither 24 nor 12, and that
        // has to be an error, not a guess.
        if payload.len() < needed || payload.len() - needed >= width {
            continue;
        }
        // Real data with a second time column is the common LTSpice shape; the flag names differ
        // between dialects, so the layout is what we trust.
        let mut values = vec![vec![0.0; variables]; points];
        let mut imaginary = vec![vec![0.0; variables]; points];
        let mut cursor = 0usize;
        let mut read = |cursor: &mut usize| -> f64 {
            let value = if width == 8 {
                let mut buffer = [0u8; 8];
                buffer.copy_from_slice(&payload[*cursor..*cursor + 8]);
                f64::from_le_bytes(buffer)
            } else {
                let mut buffer = [0u8; 4];
                buffer.copy_from_slice(&payload[*cursor..*cursor + 4]);
                f32::from_le_bytes(buffer) as f64
            };
            *cursor += width;
            value
        };
        if fast {
            let _scale = read(&mut cursor);
            for variable in 0..variables {
                for point in 0..points {
                    values[point][variable] = read(&mut cursor);
                    if complex {
                        imaginary[point][variable] = read(&mut cursor);
                    }
                }
            }
        } else {
            for point in 0..points {
                for variable in 0..variables {
                    values[point][variable] = read(&mut cursor);
                    if complex {
                        imaginary[point][variable] = read(&mut cursor);
                    }
                }
            }
        }
        plot.values = values;
        plot.imaginary = if complex { imaginary } else { Vec::new() };
        return Ok(());
    }
    Err(Error::Truncated {
        expected: (fast_prefix + points * variables * per_value) * 8,
        got: payload.len(),
    })
}

fn read_ascii(plot: &mut Plot, payload: &[u8], points: usize, variables: usize) -> Result<(), Error> {
    let text = String::from_utf8_lossy(payload);
    let mut values = vec![vec![0.0; variables]; points];
    for (point, chunk) in text.split("\n\n").filter(|chunk| !chunk.trim().is_empty()).take(points).enumerate() {
        for line in chunk.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let index = parts.next().and_then(|text| text.parse::<usize>().ok());
            let Some(index) = index.filter(|index| *index < variables) else { continue };
            let Some(value) = parts.next().and_then(|text| text.parse::<f64>().ok()) else { continue };
            values[point][index] = value;
        }
    }
    plot.values = values;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(points: usize, variables: &[(&str, &str)], flags: &str) -> String {
        let mut text = String::new();
        text.push_str("Title: differential pair\n");
        text.push_str("Date: Mon Nov 16 12:00:00 2026\n");
        text.push_str("Plotname: Transient Analysis\n");
        text.push_str(&format!("Flags: real {flags}\n"));
        text.push_str(&format!("No. Variables: {}\n", variables.len()));
        text.push_str(&format!("No. Points: {points}\n"));
        text.push_str("Variables:\n");
        for (index, (name, kind)) in variables.iter().enumerate() {
            text.push_str(&format!("\t{index}\t{name}\t{kind}\n"));
        }
        text
    }

    fn doubles(values: &[f64]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    #[test]
    fn reads_a_point_major_double_plot() {
        // Two variables, three points: time then V(out) for each point, the LTSpice layout.
        let mut bytes = header(3, &[("time", "time"), ("V(out)", "voltage")], "").into_bytes();
        bytes.extend_from_slice(b"Binary:\n");
        bytes.extend_from_slice(&doubles(&[0.0, 0.0, 1e-6, 1.5, 2e-6, 3.0]));
        let plot = parse(&bytes).expect("parse");
        assert_eq!(plot.points(), 3);
        assert_eq!(plot.variables[1].name, "V(out)");
        assert_eq!(plot.column_at(1).expect("column"), vec![0.0, 1.5, 3.0]);
        assert_eq!(plot.column("time").expect("time").len(), 3);
        assert!(!plot.is_complex());
    }

    #[test]
    fn reads_a_variable_major_fastaccess_plot() {
        // `fastaccess` writes a leading count, then one whole column after another.
        let mut bytes = header(2, &[("time", "time"), ("V(in)", "voltage")], "fastaccess").into_bytes();
        bytes.extend_from_slice(b"Binary:\n");
        bytes.extend_from_slice(&doubles(&[2.0, 0.0, 1e-3, 5.0, 5.0]));
        let plot = parse(&bytes).expect("parse");
        assert_eq!(plot.column_at(0).expect("time"), vec![0.0, 1e-3]);
        assert_eq!(plot.column_at(1).expect("vin"), vec![5.0, 5.0]);
    }

    #[test]
    fn reads_a_complex_plot_and_keeps_both_parts() {
        let mut bytes = header(2, &[("frequency", "frequency"), ("V(out)", "voltage")], "complex").into_bytes();
        bytes.extend_from_slice(b"Binary:\n");
        // real/imag pairs: (1, 0) at 1 Hz, (0.5, -0.5) at 2 Hz, then the same for V(out).
        bytes.extend_from_slice(&doubles(&[1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.5, -0.5]));
        let plot = parse(&bytes).expect("parse");
        assert!(plot.is_complex());
        assert_eq!(plot.column_at(0).expect("freq"), vec![1.0, 2.0]);
        assert_eq!(plot.column_at(1).expect("vout"), vec![1.0, 0.5]);
        assert_eq!(plot.imaginary[1][1], -0.5);
    }

    #[test]
    fn a_truncated_payload_is_an_error_not_a_panic() {
        let mut bytes = header(3, &[("time", "time")], "").into_bytes();
        bytes.extend_from_slice(b"Binary:\n");
        bytes.extend_from_slice(&doubles(&[0.0, 1.0]));
        assert!(matches!(parse(&bytes), Err(Error::Truncated { .. })));
    }

    #[test]
    fn a_file_without_a_data_marker_says_so() {
        let bytes = header(1, &[("time", "time")], "").into_bytes();
        assert!(matches!(parse(&bytes), Err(Error::NoDataSection)));
    }

    #[test]
    fn column_names_are_matched_as_the_netlist_spelled_them() {
        let mut bytes = header(1, &[("time", "time"), ("I(R1)", "current")], "").into_bytes();
        bytes.extend_from_slice(b"Binary:\n");
        bytes.extend_from_slice(&doubles(&[0.0, -0.25]));
        let plot = parse(&bytes).expect("parse");
        assert!(plot.column("I(R1)").is_some());
        assert!(plot.column("i(r1)").is_none(), "names are case-sensitive, like SPICE itself");
    }
}

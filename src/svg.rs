//! Plot a SPICE waveform file into SVG.
//!
//! Why SVG first: a plot that is a file can go into a report, a wiki, a commit message and a CI
//! artefact, and it can be checked by a test that does not need a windowing system. The interactive
//! viewer comes after, and it will draw the same curves through the same scaling code.

use crate::Plot;

/// What to draw and how big.
#[derive(Debug, Clone)]
pub struct Options {
    pub width: f64,
    pub height: f64,
    /// Column to use as the x axis; the first variable when absent.
    pub x: Option<String>,
    /// Columns to draw; every column except x when empty.
    pub series: Vec<String>,
    /// Use a logarithmic y axis.
    pub log_y: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 960.0,
            height: 540.0,
            x: None,
            series: Vec::new(),
            log_y: false,
        }
    }
}

const COLORS: [&str; 8] = [
    "#6ab7ff", "#7ee2b8", "#ffb86b", "#ff7eb6", "#c4a7ff", "#8fe388", "#7fd8d8", "#f2f2a1",
];

/// Render one plot into a self-contained SVG document.
pub fn render(plot: &Plot, options: &Options) -> Result<String, String> {
    if plot.points() == 0 || plot.variables.is_empty() {
        return Err("the plot has no data to draw".to_string());
    }
    let x_index = match &options.x {
        Some(name) => plot
            .variables
            .iter()
            .position(|variable| &variable.name == name)
            .ok_or_else(|| format!("no column named {name:?}"))?,
        None => 0,
    };
    let x = plot.column_at(x_index).ok_or("the x column is missing")?;

    let series: Vec<(String, Vec<f64>)> = if options.series.is_empty() {
        (0..plot.variables.len())
            .filter(|index| *index != x_index)
            .filter_map(|index| {
                plot.column_at(index)
                    .map(|values| (plot.variables[index].name.clone(), values))
            })
            .collect()
    } else {
        options
            .series
            .iter()
            .map(|name| {
                plot.column(name)
                    .map(|values| (name.clone(), values))
                    .ok_or_else(|| format!("no column named {name:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if series.is_empty() {
        return Err("nothing to draw: the plot has only the x column".to_string());
    }

    let margin_left = 72.0;
    let margin_right = 24.0;
    let margin_top = 48.0;
    let margin_bottom = 56.0;
    let width = options.width.max(320.0);
    let height = options.height.max(240.0);
    let inner_width = width - margin_left - margin_right;
    let inner_height = height - margin_top - margin_bottom;

    let (mut x_min, mut x_max) = (f64::MAX, f64::MIN);
    for value in &x {
        x_min = x_min.min(*value);
        x_max = x_max.max(*value);
    }
    let (mut y_min, mut y_max) = (f64::MAX, f64::MIN);
    let mut valid = 0usize;
    for (_, values) in &series {
        for value in values {
            if !value.is_finite() || (options.log_y && *value <= 0.0) {
                continue;
            }
            y_min = y_min.min(*value);
            y_max = y_max.max(*value);
            valid += 1;
        }
    }
    // `f64::MAX` and `f64::MIN` are both finite, so "nothing usable was collected" has to be
    // detected by the sentinels crossing, not by an is_finite check — otherwise the axis labels
    // come out as NaN and the curves are empty.
    if valid == 0 {
        return Err(if options.log_y {
            "every selected value is non-positive, so a logarithmic axis is impossible".to_string()
        } else {
            "no finite value to draw".to_string()
        });
    }
    if !x_min.is_finite() || !x_max.is_finite() || x_max <= x_min {
        x_max = x_min + 1.0;
    }
    if options.log_y {
        y_min = y_min.log10();
        y_max = y_max.log10();
    }
    if y_max <= y_min {
        y_max = y_min + 1.0;
    }
    let y_pad = (y_max - y_min) * 0.06;
    let (y_min, y_max) = (y_min - y_pad, y_max + y_pad);

    let sx = |value: f64| margin_left + (value - x_min) / (x_max - x_min) * inner_width;
    let sy = |value: f64| {
        let value = if options.log_y { value.log10() } else { value };
        margin_top + inner_height - (value - y_min) / (y_max - y_min) * inner_height
    };

    let mut svg = String::with_capacity(16 * 1024);
    svg.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\">\n"
    ));
    svg.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#0d1117\"/>\n");
    svg.push_str(&format!(
        "<text x=\"{:.0}\" y=\"26\" fill=\"#e6edf3\" font-family=\"sans-serif\" font-size=\"16\">{}</text>\n",
        margin_left - 34.0,
        escape(&format!("{} — {}", plot.plotname, plot.title))
    ));

    // Grid and axis labels: four divisions on each axis is what a person can read.
    for step in 0..=4 {
        let fraction = step as f64 / 4.0;
        let gx = margin_left + fraction * inner_width;
        let gy = margin_top + inner_height - fraction * inner_height;
        svg.push_str(&format!(
            "<line x1=\"{gx:.1}\" y1=\"{margin_top:.1}\" x2=\"{gx:.1}\" y2=\"{:.1}\" stroke=\"#2b3444\" stroke-width=\"1\"/>\n",
            margin_top + inner_height
        ));
        svg.push_str(&format!(
            "<line x1=\"{margin_left:.1}\" y1=\"{gy:.1}\" x2=\"{:.1}\" y2=\"{gy:.1}\" stroke=\"#2b3444\" stroke-width=\"1\"/>\n",
            margin_left + inner_width
        ));
        let x_value = x_min + fraction * (x_max - x_min);
        let y_value = if options.log_y {
            10f64.powf(y_min + fraction * (y_max - y_min))
        } else {
            y_min + fraction * (y_max - y_min)
        };
        svg.push_str(&format!(
            "<text x=\"{gx:.1}\" y=\"{:.1}\" fill=\"#96a1b0\" font-family=\"monospace\" font-size=\"11\" text-anchor=\"middle\">{}</text>\n",
            margin_top + inner_height + 18.0,
            number(x_value)
        ));
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"#96a1b0\" font-family=\"monospace\" font-size=\"11\" text-anchor=\"end\">{}</text>\n",
            margin_left - 10.0,
            gy + 4.0,
            number(y_value)
        ));
    }

    // Curves.
    for (index, (name, values)) in series.iter().enumerate() {
        let color = COLORS[index % COLORS.len()];
        let mut path = String::new();
        for (point, value) in values.iter().enumerate() {
            let Some(x_value) = x.get(point) else { break };
            if options.log_y && *value <= 0.0 {
                continue;
            }
            if !value.is_finite() || !x_value.is_finite() {
                continue;
            }
            if !path.is_empty() {
                path.push(' ');
            }
            path.push_str(&format!("{:.2},{:.2}", sx(*x_value), sy(*value)));
        }
        if !path.is_empty() {
            svg.push_str(&format!(
                "<polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"1.6\" points=\"{path}\"/>\n"
            ));
        }
        // Legend entry, top right, one line per series.
        let legend_y = margin_top + 14.0 + index as f64 * 16.0;
        svg.push_str(&format!(
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"12\" height=\"3\" fill=\"{color}\"/>\n",
            width - margin_right - 150.0,
            legend_y - 4.0
        ));
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{legend_y:.1}\" fill=\"#e6edf3\" font-family=\"monospace\" font-size=\"11\">{}</text>\n",
            width - margin_right - 132.0,
            escape(name)
        ));
    }

    // Axis titles. Units follow SPICE's convention for the variable kinds; a column of an unknown
    // kind keeps its bare name rather than inventing a unit.
    let unit_for = |kind: &str| match kind {
        "time" => Some("s"),
        "voltage" => Some("V"),
        "current" => Some("A"),
        "frequency" => Some("Hz"),
        _ => None,
    };
    let x_variable = plot.variables.get(x_index);
    let x_label = x_variable.map(|variable| match unit_for(&variable.kind) {
        Some(unit) => format!("{} ({unit})", variable.name),
        None => variable.name.clone(),
    });
    let mut kinds = series.iter().filter_map(|(name, _)| {
        plot.variables
            .iter()
            .find(|variable| &variable.name == name)
            .map(|variable| variable.kind.as_str())
    });
    let y_label = kinds.next().and_then(|first| {
        let same = kinds.all(|kind| kind == first);
        match (same, unit_for(first)) {
            (true, Some(unit)) => Some(format!("{first} ({unit})")),
            _ => None,
        }
    });
    if let Some(label) = y_label {
        svg.push_str(&format!(
            "<text x=\"18\" y=\"{mid:.1}\" fill=\"#96a1b0\" font-family=\"sans-serif\" font-size=\"12\" text-anchor=\"middle\" transform=\"rotate(-90 18 {mid:.1})\">{}</text>\n",
            escape(&label),
            mid = margin_top + inner_height / 2.0
        ));
    }

    svg.push_str(&format!(
        "<text x=\"{:.1}\" y=\"{height:.0}\" fill=\"#96a1b0\" font-family=\"monospace\" font-size=\"11\" text-anchor=\"middle\">{}  ·  {} points</text>\n",
        margin_left + inner_width / 2.0,
        escape(plot.variables.get(x_index).map(|variable| variable.name.as_str()).unwrap_or("x")),
        plot.points()
    ));
    if let Some(label) = x_label {
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.0}\" fill=\"#d0d7de\" font-family=\"sans-serif\" font-size=\"12\" text-anchor=\"middle\">{}</text>\n",
            margin_left + inner_width / 2.0,
            height - 6.0,
            escape(&label)
        ));
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

/// Numbers on an axis must stay short and must not become `0.0000000001`.
fn number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if !(1e-3..1e6).contains(&magnitude) {
        format!("{value:.2e}")
    } else if magnitude >= 100.0 {
        format!("{value:.0}")
    } else if magnitude >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Plot {
        Plot {
            title: "rc low pass & test".to_string(),
            date: "Mon Nov 16 2026".to_string(),
            flags: vec!["real".to_string()],
            plotname: "Transient Analysis".to_string(),
            variables: vec![
                crate::Variable {
                    index: 0,
                    name: "time".into(),
                    kind: "time".into(),
                },
                crate::Variable {
                    index: 1,
                    name: "V(out)".into(),
                    kind: "voltage".into(),
                },
                crate::Variable {
                    index: 2,
                    name: "V(in)".into(),
                    kind: "voltage".into(),
                },
            ],
            values: vec![
                vec![0.0, 0.0, 0.0],
                vec![1e-3, 0.63, 1.0],
                vec![2e-3, 0.86, 1.0],
            ],
            imaginary: Vec::new(),
        }
    }

    #[test]
    fn a_plot_becomes_an_svg_with_one_curve_per_series() {
        let svg = render(&sample(), &Options::default()).expect("render");
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg "));
        assert_eq!(
            svg.matches("<polyline").count(),
            2,
            "two series, two curves"
        );
        assert!(svg.contains("V(out)") && svg.contains("V(in)"));
        assert!(
            svg.contains("Transient Analysis"),
            "the plot name is in the title"
        );
        assert!(
            svg.contains("&amp;"),
            "an ampersand in the title must be escaped"
        );
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn a_missing_column_is_named_in_the_error() {
        let options = Options {
            series: vec!["V(nope)".into()],
            ..Options::default()
        };
        let error = render(&sample(), &options).unwrap_err();
        assert!(error.contains("V(nope)"), "{error}");
    }

    #[test]
    fn a_plot_with_no_curves_is_refused_instead_of_drawn_empty() {
        let mut plot = sample();
        plot.variables.truncate(1);
        plot.values = plot.values.iter().map(|row| vec![row[0]]).collect();
        assert!(render(&plot, &Options::default()).is_err());
    }

    #[test]
    fn a_logarithmic_axis_skips_non_positive_points_instead_of_drawing_nan() {
        let mut plot = sample();
        // A curve that crosses zero: the log axis must drop the bad points, not emit NaN.
        plot.values[0][1] = 0.0;
        let svg = render(
            &plot,
            &Options {
                log_y: true,
                ..Options::default()
            },
        )
        .expect("render");
        assert!(svg.contains("<polyline"), "the curve is still drawn");
        assert!(
            !svg.contains("NaN") && !svg.contains("nan"),
            "no NaN in the output"
        );
    }

    #[test]
    fn a_log_axis_of_nothing_positive_is_an_error() {
        let mut plot = sample();
        plot.values = plot
            .values
            .iter()
            .map(|row| {
                vec![0.0, 0.0, 0.0]
                    .into_iter()
                    .enumerate()
                    .map(|(at, _)| if at == 0 { row[0] } else { 0.0 })
                    .collect()
            })
            .collect();
        let error = render(
            &plot,
            &Options {
                log_y: true,
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("logarithmic"), "{error}");
    }
}

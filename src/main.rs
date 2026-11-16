//! `rawread` — look inside a SPICE `.raw` file without opening LTspice.
//!
//! ```text
//! rawread plot.raw              # title, plot, variables, points, first values of each column
//! rawread plot.raw --column V(out)   # one column as CSV, ready for a spreadsheet
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("rawread <file.raw> [--column NAME] [--csv] [--svg OUT.svg [--log]]");
        return ExitCode::SUCCESS;
    }
    let path = &args[0];
    let column = args.iter().position(|arg| arg == "--column").and_then(|at| args.get(at + 1)).cloned();
    let csv = args.iter().any(|arg| arg == "--csv");
    let svg_out = args.iter().position(|arg| arg == "--svg").and_then(|at| args.get(at + 1)).cloned();

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("rawread: {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let plot = match spice_raw::parse(&bytes) {
        Ok(plot) => plot,
        Err(error) => {
            eprintln!("rawread: {path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(target) = svg_out {
        let options = spice_raw::svg::Options { x: column.clone(), series: Vec::new(), ..spice_raw::svg::Options::default() };
        match spice_raw::svg::render(&plot, &options) {
            Ok(svg) => {
                if let Err(error) = std::fs::write(&target, svg) {
                    eprintln!("rawread: {target}: {error}");
                    return ExitCode::FAILURE;
                }
                println!("wrote {target}");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("rawread: {error}");
                return ExitCode::FAILURE;
            }
        }
    }

    match column {
        Some(name) => {
            let Some(values) = plot.column(&name) else {
                let known: Vec<&str> = plot.variables.iter().map(|variable| variable.name.as_str()).collect();
                eprintln!("rawread: no column {name:?}; this plot has {}", known.join(", "));
                return ExitCode::FAILURE;
            };
            for value in values {
                println!("{value}");
            }
        }
        None => {
            if csv {
                let header: Vec<&str> = plot.variables.iter().map(|variable| variable.name.as_str()).collect();
                println!("{}", header.join(","));
                for point in 0..plot.points() {
                    let row: Vec<String> = (0..plot.variables.len())
                        .map(|index| format!("{}", plot.values[point][index]))
                        .collect();
                    println!("{}", row.join(","));
                }
            } else {
                println!("title:     {}", plot.title);
                println!("date:      {}", plot.date);
                println!("plot:      {}", plot.plotname);
                println!("flags:     {}", plot.flags.join(" "));
                println!("points:    {}", plot.points());
                println!("variables: {}", plot.variables.len());
                for variable in &plot.variables {
                    let head: Vec<String> = (0..plot.points().min(3))
                        .map(|point| format!("{}", plot.values[point][variable.index]))
                        .collect();
                    println!("  {:<24} {:<10} {}", variable.name, variable.kind, head.join("  "));
                }
            }
        }
    }
    ExitCode::SUCCESS
}

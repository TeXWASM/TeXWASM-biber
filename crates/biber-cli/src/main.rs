//! Thin CLI binary mirroring the option surface of `bin/biber` (Perl).
//!
//! Implements both normal mode (BCF → BBL) and tool mode
//! (BibTeX → BibTeX with transformations).
//!
//! Logging: `--trace`/`--debug` set the `.blg` file verbosity.
//! `--nolog` suppresses `.blg` output entirely. `--quiet` suppresses
//! screen (stderr) output. `--logfile` and `--output-directory`
//! control where the `.blg` is written.

use anyhow::{Context, Result};
use anyxml::relaxng::RelaxNGSchema;
use anyxml::sax::{DefaultSAXHandler, XMLReader};
use std::io::Write;
use std::path::{Path, PathBuf};

use std::sync::Mutex;
use std::time::Instant;
use tracing::field::Visit;
use tracing::Subscriber;
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

/// XSD datatypes namespace declaration for RELAX NG compact syntax.
const XSD_DECL: &str = "datatypes xsd = \"http://www.w3.org/2001/XMLSchema-datatypes\"\n";
/// Compiled-in RELAX NG schema for biber.conf validation (compact syntax).
/// The original RNC omits the `datatypes xsd` declaration (trang provides it);
/// we prepend it at runtime.
const CONFIG_RNC_RAW: &str = include_str!("../../../data/schemata/config.rnc");
/// Compiled-in RELAX NG schema for .bcf control file validation (compact syntax).
const BCF_RNC_RAW: &str = include_str!("../../../data/schemata/bcf.rnc");

#[derive(Debug, Default)]
struct Cli {
    /// .bcf file (normal mode) or first .bib file (tool mode).
    bcf: Option<PathBuf>,
    /// Additional .bib files for tool mode.
    tool_inputs: Vec<PathBuf>,
    output_file: Option<PathBuf>,
    output_format: String,
    input_directory: Option<PathBuf>,
    output_directory: Option<PathBuf>,
    noconf: bool,
    nolog: bool,
    trace: bool,
    debug: bool,
    /// `--quiet` count (0 = normal, 1 = quiet, 2+ = very quiet).
    quiet: u8,
    logfile: Option<String>,
    /// Tool mode active flag.
    tool: bool,
    /// --output-safechars (encode Unicode to LaTeX macros in output).
    output_safechars: Option<bool>,
    /// --output-safecharsset (base | full | null).
    output_safecharsset: Option<String>,
    /// --validate-config: validate biber.conf and exit.
    validate_config: bool,
    /// --validate-control: validate the .bcf control file and exit.
    validate_control: bool,
    /// --wraplines[=N]: wrap .bbl lines at column N (default 80, 0 = off).
    wraplines: Option<u32>,
    /// --no-bltxml-schema: suppress biblatexml RNG schema generation.
    no_bltxml_schema: bool,
    /// --validate-bltxml: validate biblatexml output against generated RNG schema.
    validate_bltxml: bool,
    /// --no-bblxml-schema: suppress bblxml RNG schema generation.
    no_bblxml_schema: bool,
    /// --validate-bblxml: validate bblxml output against generated RNG schema.
    validate_bblxml: bool,
}

fn parse_args() -> Result<Cli> {
    let mut cli = Cli {
        output_format: "bbl".to_string(),
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--noconf" => cli.noconf = true,
            "--nolog" => cli.nolog = true,
            "--trace" | "-T" => cli.trace = true,
            "--debug" | "-D" => cli.debug = true,
            "--quiet" | "-q" => cli.quiet = cli.quiet.saturating_add(1),
            "--tool" => {
                cli.tool = true;
                if cli.output_format == "bbl" {
                    cli.output_format = "bibtex".to_string();
                }
            }
            "--output-format" => {
                cli.output_format = args.next().context("--output-format requires a value")?;
            }
            "--output-file" | "-o" => {
                cli.output_file = Some(PathBuf::from(args.next().context("-o requires a value")?));
            }
            "--output-directory" => {
                cli.output_directory = Some(PathBuf::from(
                    args.next().context("--output-directory requires a value")?,
                ));
            }
            "--input-directory" => {
                cli.input_directory = Some(PathBuf::from(
                    args.next().context("--input-directory requires a value")?,
                ));
            }
            "--logfile" => {
                cli.logfile = Some(args.next().context("--logfile requires a value")?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("TeXWASM Biber - A Rust port of the Biber bibliography processor {}", env!("CARGO_PKG_VERSION"));
                println!("BCF -> BBL/BBLXML/DOT + tool mode (BibTeX/BiblateXML -> BibTeX/BiblateXML/BBLXML/DOT).");
                std::process::exit(0);
            }
            "--output-safechars" => {
                cli.output_safechars = Some(true);
            }
            "--no-output-safechars" => {
                cli.output_safechars = Some(false);
            }
            "--output-safecharsset" => {
                cli.output_safecharsset = Some(
                    args.next()
                        .context("--output-safecharsset requires a value (base|full|null)")?,
                );
            }
            "--validate-config" => cli.validate_config = true,
            "--validate-control" => cli.validate_control = true,
            "--no-bltxml-schema" => cli.no_bltxml_schema = true,
            "--validate-bltxml" => cli.validate_bltxml = true,
            "--no-bblxml-schema" => cli.no_bblxml_schema = true,
            "--validate-bblxml" => cli.validate_bblxml = true,
            "--wraplines" => {
                cli.wraplines = Some(80);
            }
            other if other.starts_with("--wraplines=") => {
                let val = other.trim_start_matches("--wraplines=");
                if val.is_empty() {
                    cli.wraplines = Some(80);
                } else {
                    cli.wraplines = Some(
                        val.parse()
                            .context("--wraplines requires a non-negative integer")?,
                    );
                }
            }
            "-w" => {
                cli.wraplines = Some(80);
            }
            other if other.starts_with("-w") && other.len() > 2 => {
                let val = &other[2..];
                cli.wraplines = Some(val.parse().context("-w requires a non-negative integer")?);
            }
            other if other.starts_with("--output-format=") => {
                cli.output_format = other.trim_start_matches("--output-format=").to_string();
            }
            other if other.starts_with("--output-file=") => {
                cli.output_file = Some(PathBuf::from(other.trim_start_matches("--output-file=")));
            }
            other if other.starts_with("--logfile=") => {
                cli.logfile = Some(other.trim_start_matches("--logfile=").to_string());
            }
            other if other.starts_with("--output-safecharsset=") => {
                cli.output_safecharsset = Some(
                    other
                        .trim_start_matches("--output-safecharsset=")
                        .to_string(),
                );
            }
            other if other.starts_with("--output-directory=") => {
                cli.output_directory = Some(PathBuf::from(
                    other.trim_start_matches("--output-directory="),
                ));
            }
            other if other.starts_with('-') => {
                anyhow::bail!("unknown option: {other}");
            }
            other => {
                if cli.bcf.is_none() {
                    cli.bcf = Some(PathBuf::from(other));
                } else if cli.tool {
                    cli.tool_inputs.push(PathBuf::from(other));
                } else {
                    anyhow::bail!("unexpected positional argument: {other}");
                }
            }
        }
    }

    // --trace implies --debug
    if cli.trace {
        cli.debug = true;
    }

    if cli.tool {
        match cli.output_format.as_str() {
            "bibtex" | "biblatexml" | "bblxml" | "dot" => {}
            other => {
                anyhow::bail!(
                    "Tool mode only supports --output-format=bibtex|biblatexml|bblxml|dot (got {})",
                    other
                );
            }
        }
    } else {
        match cli.output_format.as_str() {
            "bbl" | "bblxml" | "dot" => {}
            other => {
                anyhow::bail!(
                    "Normal mode only supports --output-format=bbl|bblxml|dot (got {})",
                    other
                );
            }
        }
    }
    Ok(cli)
}

fn print_help() {
    eprintln!(
        "TeXWASM Biber - A Rust port of the Biber bibliography processor\n\n\
         Usage:\n  \
           biber [OPTIONS] <BCF>                  (normal mode)\n  \
           biber --tool [OPTIONS] <BIB> [BIB...]  (tool mode)\n  \
           biber --validate-config                (validate biber.conf and exit)\n  \
           biber --validate-control <BCF>         (validate BCF and exit)\n\n\
         Options:\n  \
           --output-format=FMT     output format (normal: bbl|bblxml|dot; tool: bibtex|biblatexml|bblxml|dot)\n  \
           --output-file=PATH      write output to PATH\n  \
           --output-directory=DIR  write .blg log to DIR\n  \
           --input-directory=DIR   resolve datasources relative to DIR\n  \
           --logfile=NAME          base name for .blg log file\n  \
           --output-safechars      encode Unicode to LaTeX macros in output\n  \
           --no-output-safechars   disable output-safechars\n  \
           --output-safecharsset=S safechars set: base (default), full, null\n  \
           --trace, -T             trace-level logging (very verbose)\n  \
           --debug, -D             debug-level logging\n  \
           --quiet, -q             suppress screen output (repeat for less)\n  \
           --nolog                 suppress .blg log file output\n  \
           --noconf                ignore biber.conf\n  \
            --tool                  run in tool mode (BibTeX/BiblateXML -> various formats)\n  \
            --wraplines[=N] | -w[N] wrap .bbl output lines at column N (default 80, 0=off)\n  \
              --validate-config       validate biber.conf against RELAX NG schema and exit\n  \
              --validate-control      validate BCF control file against RELAX NG schema and exit\n  \
              --no-bltxml-schema      suppress generation of biblatexml RNG schema\n  \
              --validate-bltxml       validate biblatexml output against RNG schema\n  \
              --no-bblxml-schema      suppress generation of bblxml RNG schema\n  \
              --validate-bblxml       validate bblxml output against RNG schema\n \
            -h, --help              show this help\n  \
           -V, --version           show version\n"
    );
}

fn main() -> Result<()> {
    let cli = parse_args()?;

    // Derive .blg path.
    let blg_path = derive_blg_path(&cli);

    // Create .blg file if not suppressed.
    let blg_layer: Option<BlgLayer> = if !cli.nolog {
        if let Some(ref path) = blg_path {
            match std::fs::File::create(path) {
                Ok(f) => {
                    let layer = BlgLayer::new(f);
                    layer.write_header();
                    Some(layer)
                }
                Err(e) => {
                    eprintln!("Warning: cannot create {}: {e}", path.display());
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Determine screen log level.
    let screen_level = if cli.quiet >= 2 {
        "off"
    } else if cli.quiet >= 1 {
        "error"
    } else {
        "info"
    };

    // Determine file log level.
    let file_level = if cli.trace {
        "trace"
    } else if cli.debug {
        "debug"
    } else {
        "info"
    };

    // Set up dual tracing subscriber: stderr + optional .blg file.
    let screen_env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(screen_level));

    match blg_layer {
        Some(layer) => {
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .with_target(true)
                        .without_time()
                        .with_filter(screen_env_filter),
                )
                .with(layer.with_filter(tracing_subscriber::EnvFilter::new(file_level)))
                .init();
        }
        None => {
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr)
                        .with_target(true)
                        .without_time()
                        .with_filter(screen_env_filter),
                )
                .init();
        }
    }

    // --validate-config / --validate-control: validate and exit.
    if cli.validate_config {
        return validate_config(cli.input_directory.as_ref(), cli.noconf);
    }
    if cli.validate_control {
        let bcf_path = cli
            .bcf
            .as_deref()
            .context("--validate-control requires a .bcf file")?;
        return validate_control(bcf_path);
    }

    if cli.tool {
        run_tool_mode(cli)
    } else {
        run_normal_mode(cli)
    }
}

// ---------------------------------------------------------------------------
// .blg tracing layer -- writes `[%r] target> LEVEL - msg` to a file
// ---------------------------------------------------------------------------

struct BlgLayer {
    writer: Mutex<std::io::BufWriter<std::fs::File>>,
    start: Instant,
}

impl BlgLayer {
    fn new(file: std::fs::File) -> Self {
        Self {
            writer: Mutex::new(std::io::BufWriter::new(file)),
            start: Instant::now(),
        }
    }

    /// Write the `.blg` header line (`=== <datetime>`).
    fn write_header(&self) {
        let now = chrono_like_now();
        let header = format!("=== {now}\n");
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(header.as_bytes());
            let _ = w.flush();
        }
    }

    /// Flush the internal buffer.
    fn flush(&self) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.flush();
        }
    }
}

impl<S: Subscriber> Layer<S> for BlgLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let elapsed = self.start.elapsed().as_millis() as u64;

        // Extract target (module path) from metadata.
        let meta = event.metadata();
        let target = meta.target();
        // Shorten: take last component after `::`.
        let short_target = target.rsplit("::").next().unwrap_or(target);

        let level = match *meta.level() {
            tracing::Level::TRACE => "TRACE",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::INFO => "INFO",
            tracing::Level::WARN => "WARN",
            tracing::Level::ERROR => "ERROR",
        };

        // Collect the message via a visitor.
        struct MsgVisitor(String);
        impl Visit for MsgVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut visitor = MsgVisitor(String::new());
        event.record(&mut visitor);
        let msg = visitor.0;
        if msg.is_empty() {
            return;
        }

        // Format: [%r] target> LEVEL - msg\n
        let line = format!("[{:5}] {}> {} - {}\n", elapsed, short_target, level, msg);

        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.flush();
        }
    }

    fn on_close(&self, _id: tracing::span::Id, _ctx: LayerContext<'_, S>) {
        self.flush();
    }
}

// ---------------------------------------------------------------------------
// Date/time formatting (no chrono dependency)
// ---------------------------------------------------------------------------

/// Format a datetime string like Perl's `strftime "%a %b %e, %Y, %H:%M:%S"`.
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day, hour, min, sec) = unix_to_calendar(secs);
    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let day_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let dow = day_of_week(year, month, day);
    format!(
        "{} {} {:2}, {}, {:02}:{:02}:{:02}",
        day_names[dow],
        month_names[(month - 1) as usize],
        day,
        year,
        hour,
        min,
        sec
    )
}

fn unix_to_calendar(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let mut days = secs / 86400;
    let tod = secs % 86400;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;
    let mut y = 1970u64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0u64;
    while m < 12 && days >= mdays[m as usize] {
        days -= mdays[m as usize];
        m += 1;
    }
    (y, m + 1, days + 1, hour, min, sec)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn day_of_week(y: u64, m: u64, d: u64) -> usize {
    let q = d;
    let m_adj = if m < 3 { m + 12 } else { m };
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m_adj + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // h: 0=Sat, 1=Sun, ..., 6=Fri -> convert to 0=Sun
    ((h + 6) % 7) as usize
}

// ---------------------------------------------------------------------------
// .blg path derivation
// ---------------------------------------------------------------------------

/// Derive the `.blg` file path from the BCF path or --logfile option.
fn derive_blg_path(cli: &Cli) -> Option<PathBuf> {
    // --nolog -> no file at all.
    if cli.nolog {
        return None;
    }

    // --logfile takes precedence.
    if let Some(ref name) = cli.logfile {
        let mut path = PathBuf::from(name);
        if path.extension().is_none() {
            path.set_extension("blg");
        }
        if let Some(ref dir) = cli.output_directory {
            return Some(dir.join(path));
        }
        return Some(path);
    }

    // Derive from BCF path (normal mode) or first input (tool mode).
    let base = if let Some(ref bcf) = cli.bcf {
        bcf.file_stem().map(|s| {
            let mut p = PathBuf::from(s);
            p.set_extension("blg");
            p
        })
    } else if let Some(first) = cli.tool_inputs.first() {
        first.file_stem().map(|s| {
            let mut p = PathBuf::from(s);
            p.set_extension("blg");
            p
        })
    } else {
        None
    };

    base.map(|p| {
        if let Some(ref dir) = cli.output_directory {
            dir.join(p)
        } else {
            p
        }
    })
}

/// Try to find and load `biber.conf`.
///
/// Searches the current directory and `input_directory` (if given).
fn maybe_load_biber_config(
    config: &mut biber_core::Config,
    noconf: bool,
    input_directory: Option<&PathBuf>,
) {
    if noconf {
        return;
    }
    let search_dirs: Vec<PathBuf> = {
        let mut dirs = Vec::new();
        dirs.push(PathBuf::from("."));
        if let Some(input_dir) = input_directory {
            dirs.push(input_dir.to_path_buf());
        }
        dirs
    };
    for dir in &search_dirs {
        let conf_path = dir.join("biber.conf");
        if conf_path.exists() {
            match std::fs::read_to_string(&conf_path) {
                Ok(text) => match biber_core::parse_biber_config(&text, config) {
                    Ok(()) => {
                        tracing::info!("Loaded config: {}", conf_path.display());
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse {}: {e}", conf_path.display());
                        return;
                    }
                },
                Err(e) => {
                    tracing::warn!("Cannot read {}: {e}", conf_path.display());
                    return;
                }
            }
        }
    }
}

/// Apply CLI logging flags to the Config options.
fn apply_logging_opts(config: &mut biber_core::Config, cli: &Cli) {
    if cli.trace {
        config.setoption_str("trace", "1");
        config.mark_explicit("trace");
    }
    if cli.debug {
        config.setoption_str("debug", "1");
        config.mark_explicit("debug");
    }
    if cli.quiet > 0 {
        config.setoption_str("quiet", "1");
        config.mark_explicit("quiet");
    }
    if cli.nolog {
        config.setoption_str("nolog", "1");
        config.mark_explicit("nolog");
    }
    if let Some(ref name) = cli.logfile {
        config.setoption_str("logfile", name);
        config.mark_explicit("logfile");
    }
}

/// Apply CLI safechars flags to the Config options.
fn apply_safechars_opts(config: &mut biber_core::Config, cli: &Cli) {
    if let Some(v) = cli.output_safechars {
        config.setoption_str("output_safechars", if v { "1" } else { "0" });
        config.mark_explicit("output_safechars");
    }
    if let Some(ref set) = cli.output_safecharsset {
        config.setoption_str("output_safecharsset", set);
        config.mark_explicit("output_safecharsset");
    }
}

/// Apply CLI wraplines flag to the Config options.
fn apply_wraplines_opts(config: &mut biber_core::Config, cli: &Cli) {
    if let Some(v) = cli.wraplines {
        config.setoption_str("wraplines", v.to_string());
        config.mark_explicit("wraplines");
    }
}

/// Apply CLI schema option flags to the Config options.
fn apply_schema_opts(config: &mut biber_core::Config, cli: &Cli) {
    if cli.no_bltxml_schema {
        config.setoption_str("no_bltxml_schema", "1");
        config.mark_explicit("no_bltxml_schema");
    }
    if cli.validate_bltxml {
        config.setoption_str("validate_bltxml", "1");
        config.mark_explicit("validate_bltxml");
    }
    if cli.no_bblxml_schema {
        config.setoption_str("no_bblxml_schema", "1");
        config.mark_explicit("no_bblxml_schema");
    }
    if cli.validate_bblxml {
        config.setoption_str("validate_bblxml", "1");
        config.mark_explicit("validate_bblxml");
    }
}

/// Run normal mode: read BCF, resolve .bib datasources, run pipeline, emit BBL.
fn run_normal_mode(cli: Cli) -> Result<()> {
    let bcf_path = cli.bcf.as_deref().context("no .bcf file given")?;
    let bcf_text = std::fs::read_to_string(bcf_path)
        .with_context(|| format!("reading {}", bcf_path.display()))?;

    let mut biber = biber_input_bcf::parse_bcf(&bcf_text)
        .map_err(|e| anyhow::anyhow!("BCF parse error: {e}"))?;

    // Apply CLI logging flags.
    apply_logging_opts(&mut biber.config, &cli);
    // Apply CLI safechars flags (after config file so CLI takes precedence).
    apply_safechars_opts(&mut biber.config, &cli);
    // Apply CLI wraplines flag.
    apply_wraplines_opts(&mut biber.config, &cli);
    // Apply CLI schema option flags.
    apply_schema_opts(&mut biber.config, &cli);

    // Load biber.conf (BCF options will override, which is correct)
    maybe_load_biber_config(&mut biber.config, cli.noconf, cli.input_directory.as_ref());

    tracing::info!(
        "Parsed BCF: {} sections, {} datalists",
        biber.sections.len(),
        biber.datalists.len()
    );

    let bcf_dir = bcf_path.parent().unwrap_or(std::path::Path::new("."));
    for section in biber.sections.get_sections_mut() {
        let datasources: Vec<_> = section.get_datasources().to_vec();
        for ds_ref in &datasources {
            if ds_ref.r#type != "file" {
                continue;
            }
            let ds_path = if ds_ref.name.is_empty() {
                continue;
            } else if std::path::Path::new(&ds_ref.name).is_absolute() {
                PathBuf::from(&ds_ref.name)
            } else {
                bcf_dir.join(&ds_ref.name)
            };

            let bib_text = match std::fs::read_to_string(&ds_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("cannot read {}: {e}", ds_path.display());
                    continue;
                }
            };

            let (map, order, _preambles) = biber_input_bib::parse_bib_into_map(&bib_text)
                .map_err(|e| anyhow::anyhow!("BibTeX parse error in {}: {e}", ds_path.display()))?;

            tracing::info!("Parsed {}: {} entries", ds_path.display(), map.len());

            // Extract IDS aliases
            for key in order.iter() {
                if let Some(bib_entry) = map.get(key) {
                    if let Some(ids) = bib_entry.get("ids") {
                        for alias in ids.split(',').map(|s| s.trim().to_string()) {
                            if !alias.is_empty() && alias != *key {
                                section.set_citekey_alias(&alias, key);
                                if section.get_citekeys().contains(&alias) {
                                    section.add_citekeys(std::iter::once(key.clone()));
                                }
                            }
                        }
                    }
                }
            }

            // Add entries to the section
            for key in &order {
                let is_cited = section.get_citekeys().contains(key);
                let is_alias_target = section
                    .get_citekeys()
                    .iter()
                    .any(|ck| section.get_citekey_alias(ck) == Some(key.as_str()));
                if (is_cited || is_alias_target || section.is_allkeys())
                    && !section.bibentries.has_entry(key)
                {
                    let bib_entry = &map[key];
                    let mut entry = biber_core::Entry::new(key.clone(), bib_entry.typ.clone());
                    entry.datasource = ds_ref.name.clone();
                    entry.set_field_str("citekey", key);
                    entry.set_field_str("entrytype", &bib_entry.typ);
                    for (field, value) in &bib_entry.fields {
                        entry.set_field_str(field, value);
                    }
                    entry.set_field_str("datatype", "bibtex");
                    section.bibentries.add_entry(entry);
                }
            }

            // Mark citekeys not found in any datasource as undefined
            let cited_keys: Vec<String> = section.get_citekeys().to_vec();
            for ck in &cited_keys {
                if !section.bibentries.has_entry(ck) && section.get_citekey_alias(ck).is_none() {
                    section.add_undef_citekey(ck);
                }
            }
        }
    }

    let total: usize = biber
        .sections
        .get_sections()
        .iter()
        .map(|s| s.bibentries.len())
        .sum();
    tracing::info!("Total entries loaded: {total}");

    biber_core::pipeline::prepare(&mut biber);
    tracing::info!("Pipeline complete");

    let output_format = cli.output_format.as_str();
    let output: String = match output_format {
        "bblxml" => {
            tracing::info!("bblxml output generated");
            biber_output_bblxml::write_bblxml(&biber)
        }
        "dot" => {
            tracing::info!("DOT output generated");
            biber_output_dot::write_dot(&biber)
        }
        _ => {
            // Default to bbl
            tracing::info!("BBL output generated");
            biber_output_bbl::write_bbl(&biber)
        }
    };

    // RNG schema generation and validation for bblxml
    if output_format == "bblxml" && biber.config.getoption_str("no_bblxml_schema") != Some("1") {
        let schema = biber.datamodel.generate_bblxml_schema();
        let schema_path = derive_schema_path(cli.output_file.as_deref(), "bblxml.rng");
        if let Some(ref path) = schema_path {
            if let Err(e) = std::fs::write(path, &schema) {
                tracing::warn!("Cannot write schema file {}: {e}", path.display());
            } else {
                tracing::info!("Wrote bblxml RNG schema to {}", path.display());
            }
        }
        if biber.config.getoption_str("validate_bblxml") == Some("1") {
            match validate_xml_with_rng(&output, &schema, "bblxml.rng") {
                Ok(()) => tracing::info!("bblxml validation passed"),
                Err(e) => tracing::warn!("bblxml validation failed: {e}"),
            }
        }
    }

    match cli.output_file.as_deref() {
        Some(path) => {
            std::fs::write(path, output.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        None => {
            std::io::stdout().write_all(output.as_bytes())?;
        }
    }
    Ok(())
}

/// Tool input format: BibTeX or biblatexml.
enum InputFormat {
    Bibtex,
    Biblatexml,
}

/// Detect input format from file extension.
fn detect_input_format(path: &Path) -> InputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("bltxml") => InputFormat::Biblatexml,
        _ => InputFormat::Bibtex,
    }
}

/// Run tool mode: read datasource files, run pipeline, emit output.
fn run_tool_mode(cli: Cli) -> Result<()> {
    let input_paths: Vec<PathBuf> = {
        let mut paths = Vec::new();
        if let Some(bcf) = &cli.bcf {
            paths.push(bcf.clone());
        }
        paths.extend(cli.tool_inputs.clone());
        paths
    };

    if input_paths.is_empty() {
        anyhow::bail!("tool mode requires at least one input file (.bib or .bltxml)");
    }

    let tool_input_dir = cli
        .input_directory
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let output_format = cli.output_format.clone();

    // Create a Biber instance with section 99999 and allkeys=true
    let mut biber = biber_core::processor::Biber::new();
    let mut section = biber_core::section::Section::new(99999);
    section.set_allkeys(true);

    // Load biber.conf (tool mode uses it for defaults)
    maybe_load_biber_config(&mut biber.config, cli.noconf, cli.input_directory.as_ref());

    // Apply CLI logging flags.
    apply_logging_opts(&mut biber.config, &cli);
    // Apply CLI safechars flags (after config file so CLI takes precedence).
    apply_safechars_opts(&mut biber.config, &cli);
    // Apply CLI wraplines flag.
    apply_wraplines_opts(&mut biber.config, &cli);
    // Apply CLI schema option flags.
    apply_schema_opts(&mut biber.config, &cli);

    // Set tool mode config (CLI overrides take precedence over biber.conf)
    biber.config.setoption_str("tool", "1");
    biber.config.setoption_str("output_format", &output_format);
    biber.config.mark_explicit("tool");
    biber.config.mark_explicit("output_format");

    for input_path in &input_paths {
        let resolved = if input_path.is_absolute() {
            input_path.clone()
        } else {
            tool_input_dir.join(input_path)
        };

        let text = std::fs::read_to_string(&resolved)
            .with_context(|| format!("reading {}", resolved.display()))?;

        let fmt = detect_input_format(input_path);

        match fmt {
            InputFormat::Bibtex => {
                let (map, order, _preambles) =
                    biber_input_bib::parse_bib_into_map(&text).map_err(|e| {
                        anyhow::anyhow!("BibTeX parse error in {}: {e}", resolved.display())
                    })?;

                tracing::info!(
                    "Tool mode: parsed {}: {} entries",
                    resolved.display(),
                    map.len()
                );

                section.add_datasource(biber_core::section::DatasourceRef {
                    r#type: "file".into(),
                    name: resolved.to_string_lossy().into_owned(),
                    datatype: "bibtex".into(),
                    encoding: Some("UTF-8".into()),
                    glob: false,
                });

                for key in &order {
                    if let Some(bib_entry) = map.get(key) {
                        if section.bibentries.has_entry(key) {
                            continue;
                        }
                        let mut entry = biber_core::Entry::new(key.clone(), bib_entry.typ.clone());
                        entry.datasource = input_path.to_string_lossy().into_owned();
                        entry.set_field_str("citekey", key);
                        entry.set_field_str("entrytype", &bib_entry.typ);
                        for (field, value) in &bib_entry.fields {
                            entry.set_field_str(field, value);
                        }
                        entry.set_field_str("datatype", "bibtex");
                        section.bibentries.add_entry(entry);
                        section.add_citekeys(std::iter::once(key.clone()));
                    }
                }
            }
            InputFormat::Biblatexml => {
                let (map, order, _preambles) = biber_input_biblatexml::parse_bltxml_into_map(&text)
                    .map_err(|e| {
                        anyhow::anyhow!("BiblateXML parse error in {}: {e}", resolved.display())
                    })?;

                tracing::info!(
                    "Tool mode: parsed {}: {} entries",
                    resolved.display(),
                    map.len()
                );

                section.add_datasource(biber_core::section::DatasourceRef {
                    r#type: "file".into(),
                    name: resolved.to_string_lossy().into_owned(),
                    datatype: "biblatexml".into(),
                    encoding: Some("UTF-8".into()),
                    glob: false,
                });

                for key in &order {
                    if let Some(bltx_entry) = map.get(key) {
                        if section.bibentries.has_entry(key) {
                            continue;
                        }
                        let mut entry = biber_core::Entry::new(key.clone(), bltx_entry.typ.clone());
                        entry.datasource = input_path.to_string_lossy().into_owned();
                        entry.set_field_str("citekey", key);
                        entry.set_field_str("entrytype", &bltx_entry.typ);
                        for (field, value) in &bltx_entry.fields {
                            entry.set_field_str(field, value);
                        }
                        entry.set_field_str("datatype", "biblatexml");
                        section.bibentries.add_entry(entry);
                        section.add_citekeys(std::iter::once(key.clone()));
                    }
                }
            }
        }
    }

    biber.sections.add_section(section);

    let total: usize = biber
        .sections
        .get_sections()
        .iter()
        .map(|s| s.bibentries.len())
        .sum();
    tracing::info!("Tool mode: total entries loaded: {total}");

    // Run the tool-mode pipeline
    biber_core::pipeline::prepare_tool(&mut biber);
    tracing::info!("Tool mode: pipeline complete");

    // Generate output in the requested format
    let output: String = match output_format.as_str() {
        "biblatexml" => {
            tracing::info!("Tool mode: biblatexml output generated");
            biber_output_biblatexml::write_bltxml(&biber)
        }
        "bblxml" => {
            tracing::info!("Tool mode: bblxml output generated");
            biber_output_bblxml::write_bblxml(&biber)
        }
        "dot" => {
            tracing::info!("Tool mode: DOT output generated");
            biber_output_dot::write_dot(&biber)
        }
        _ => {
            tracing::info!("Tool mode: BibTeX output generated");
            biber_output_bibtex::write_bib(&biber)
        }
    };

    // RNG schema generation and validation
    match output_format.as_str() {
        "biblatexml" if biber.config.getoption_str("no_bltxml_schema") != Some("1") => {
            let schema = biber.datamodel.generate_bltxml_schema();
            let schema_path = derive_schema_path(cli.output_file.as_deref(), "biblatexml.rng");
            if let Some(ref path) = schema_path {
                if let Err(e) = std::fs::write(path, &schema) {
                    tracing::warn!("Cannot write schema file {}: {e}", path.display());
                } else {
                    tracing::info!("Wrote biblatexml RNG schema to {}", path.display());
                }
            }
            if biber.config.getoption_str("validate_bltxml") == Some("1") {
                match validate_xml_with_rng(&output, &schema, "biblatexml.rng") {
                    Ok(()) => tracing::info!("biblatexml validation passed"),
                    Err(e) => tracing::warn!("biblatexml validation failed: {e}"),
                }
            }
        }
        "bblxml" if biber.config.getoption_str("no_bblxml_schema") != Some("1") => {
            let schema = biber.datamodel.generate_bblxml_schema();
            let schema_path = derive_schema_path(cli.output_file.as_deref(), "bblxml.rng");
            if let Some(ref path) = schema_path {
                if let Err(e) = std::fs::write(path, &schema) {
                    tracing::warn!("Cannot write schema file {}: {e}", path.display());
                } else {
                    tracing::info!("Wrote bblxml RNG schema to {}", path.display());
                }
            }
            if biber.config.getoption_str("validate_bblxml") == Some("1") {
                match validate_xml_with_rng(&output, &schema, "bblxml.rng") {
                    Ok(()) => tracing::info!("bblxml validation passed"),
                    Err(e) => tracing::warn!("bblxml validation failed: {e}"),
                }
            }
        }
        _ => {}
    };

    match cli.output_file.as_deref() {
        Some(path) => {
            std::fs::write(path, output.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        None => {
            std::io::stdout().write_all(output.as_bytes())?;
        }
    }
    Ok(())
}

/// Derive the schema file path from the output file path.
/// Falls back to a default name if no output file is specified.
fn derive_schema_path(output_file: Option<&Path>, _default_name: &str) -> Option<PathBuf> {
    match output_file {
        Some(path) => {
            let mut p = path.to_path_buf();
            p.set_extension("rng");
            Some(p)
        }
        None => None,
    }
}

/// Workaround for `anyxml-automata` v0.1.3 bug: `parse_multi_char_esc()` has an
/// inverted condition that rejects `\d`, `\S` etc. Replace known multi-character
/// escapes with equivalent character classes before schema parsing.
fn relaxng_workaround(schema: &str) -> String {
    schema
        .replace("\\d", "[0-9]")
        .replace("\\S", "[^\\t\\n\\r]")
        .replace("\\s", "[\\t\\n\\r ]")
}

/// Validate an XML document against a compiled-in RELAX NG schema (RNG XML format).
/// The schema is parsed as RNG XML (not compact syntax).
fn validate_xml_with_rng(xml: &str, schema: &str, schema_label: &str) -> Result<()> {
    let mut rng = match RelaxNGSchema::parse_str(schema, None, Some(DefaultSAXHandler)) {
        Ok(rng) => rng,
        Err(e) => {
            anyhow::bail!("failed to parse RELAX NG schema {schema_label}: {e:?}");
        }
    };
    let validator = rng.new_validate_handler(DefaultSAXHandler);
    let mut reader = XMLReader::builder().set_handler(validator).build();
    reader
        .parse_str(xml, None)
        .map_err(|e| anyhow::anyhow!("XML parse error: {e:?}"))?;
    if let Err(e) = &reader.handler.last_error {
        anyhow::bail!("validation against {schema_label} failed: {e}");
    }
    Ok(())
}

/// Validate an XML document against a compiled-in RELAX NG schema.
/// Accepts either compact (RNC) or XML (RNG) syntax.
fn validate_xml_with_rnc(
    xml: &str,
    schema_raw: &str,
    ns_decl: &str,
    schema_label: &str,
) -> Result<()> {
    // Prepend namespace declaration (the original RNC files omit datatypes xsd)
    let schema_text = if ns_decl.is_empty() {
        schema_raw.to_string()
    } else {
        format!("{ns_decl}{schema_raw}")
    };
    // Apply workaround for anyxml-automata bug
    let schema_text = relaxng_workaround(&schema_text);
    let mut rng =
        match RelaxNGSchema::parse_compact_str(&schema_text, None, Some(DefaultSAXHandler)) {
            Ok(rng) => rng,
            Err(e) => {
                anyhow::bail!("failed to parse RELAX NG schema {schema_label}: {e:?}");
            }
        };
    let validator = rng.new_validate_handler(DefaultSAXHandler);
    let mut reader = XMLReader::builder().set_handler(validator).build();
    reader
        .parse_str(xml, None)
        .map_err(|e| anyhow::anyhow!("XML parse error: {e:?}"))?;
    if let Err(e) = &reader.handler.last_error {
        anyhow::bail!("validation against {schema_label} failed: {e}");
    }
    Ok(())
}

/// Validate `biber.conf` against the RELAX NG schema and report results.
fn validate_config(input_directory: Option<&PathBuf>, noconf: bool) -> Result<()> {
    if noconf {
        eprintln!("--noconf enabled, skipping config validation");
        return Ok(());
    }

    let search_dirs: Vec<PathBuf> = {
        let mut dirs = Vec::new();
        dirs.push(PathBuf::from("."));
        if let Some(input_dir) = input_directory {
            dirs.push(input_dir.to_path_buf());
        }
        dirs
    };

    for dir in &search_dirs {
        let conf_path = dir.join("biber.conf");
        if conf_path.exists() {
            eprintln!("Validating config: {}", conf_path.display());
            match std::fs::read_to_string(&conf_path) {
                Ok(text) => {
                    // RNC schema validation
                    if let Err(e) =
                        validate_xml_with_rnc(&text, CONFIG_RNC_RAW, XSD_DECL, "config.rnc")
                    {
                        eprintln!("  Config: FAILED (schema)");
                        eprintln!("  {}", e);
                        std::process::exit(1);
                    }
                    // Semantic validation via config parser
                    let mut config = biber_core::Config::new();
                    match biber_core::parse_biber_config(&text, &mut config) {
                        Ok(()) => {
                            eprintln!("  Config: OK (RNG schema + parse)");
                        }
                        Err(e) => {
                            eprintln!("  Config: FAILED (semantic)");
                            eprintln!("  Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Cannot read {}: {e}", conf_path.display());
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
    }

    eprintln!("No biber.conf found in search paths");
    std::process::exit(1);
}

/// Validate a `.bcf` control file against the RELAX NG schema and report results.
fn validate_control(bcf_path: &Path) -> Result<()> {
    eprintln!("Validating control file: {}", bcf_path.display());
    let bcf_text = std::fs::read_to_string(bcf_path)
        .with_context(|| format!("reading {}", bcf_path.display()))?;

    // RNC schema validation
    if let Err(e) = validate_xml_with_rnc(&bcf_text, BCF_RNC_RAW, XSD_DECL, "bcf.rnc") {
        eprintln!("  BCF: FAILED (schema)");
        eprintln!("  {}", e);
        std::process::exit(1);
    }

    // Structural validation via BCF parser
    match biber_input_bcf::parse_bcf(&bcf_text) {
        Ok(biber) => {
            eprintln!(
                "  BCF: OK (RNG schema + parse, version {}, {} sections, {} datalists)",
                biber.config.getoption_str("controlversion").unwrap_or("?"),
                biber.sections.len(),
                biber.datalists.len()
            );
        }
        Err(e) => {
            eprintln!("  BCF: FAILED (parse)");
            eprintln!("  Error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

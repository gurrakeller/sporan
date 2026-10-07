//! Speran CLI: runner, static checker, and REPL.

use speran::error::SperanError;
use speran::interp::Limits;
use speran::tools::ToolRegistry;
use speran::{RunReport, SperanRunner};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_help();
        return;
    }

    let mut file: Option<String> = None;
    let mut eval_code: Option<String> = None;
    let mut tools_file: Option<String> = None;
    let mut grants_str: Option<String> = None;
    let mut check_only = false;
    let mut json_output = false;
    let mut repl_mode = false;
    let mut max_steps: Option<usize> = None;
    let mut max_tool_calls: Option<usize> = None;
    let mut timeout_ms: Option<u64> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-v" | "--version" => {
                println!("speran v{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-e" | "--eval" => {
                i += 1;
                if i < args.len() {
                    eval_code = Some(args[i].clone());
                } else {
                    eprintln!("Error: missing code string after -e/--eval");
                    process::exit(1);
                }
            }
            "--tools" => {
                i += 1;
                if i < args.len() {
                    tools_file = Some(args[i].clone());
                } else {
                    eprintln!("Error: missing file path after --tools");
                    process::exit(1);
                }
            }
            "--grants" => {
                i += 1;
                if i < args.len() {
                    grants_str = Some(args[i].clone());
                } else {
                    eprintln!("Error: missing comma-separated list after --grants");
                    process::exit(1);
                }
            }
            "--check" => {
                check_only = true;
            }
            "--json" => {
                json_output = true;
            }
            "--repl" => {
                repl_mode = true;
            }
            "--max-steps" => {
                i += 1;
                if i < args.len() {
                    max_steps = args[i].parse().ok();
                }
            }
            "--max-tools" => {
                i += 1;
                if i < args.len() {
                    max_tool_calls = args[i].parse().ok();
                }
            }
            "--timeout" => {
                i += 1;
                if i < args.len() {
                    timeout_ms = args[i].parse().ok();
                }
            }
            arg if !arg.starts_with('-') && file.is_none() => {
                file = Some(arg.to_string());
            }
            other => {
                eprintln!("Error: unrecognized argument '{}'", other);
                process::exit(1);
            }
        }
        i += 1;
    }

    if repl_mode {
        run_repl();
        return;
    }

    let source = if let Some(code) = eval_code {
        code
    } else if let Some(path) = file {
        match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error: unable to read file '{}': {}", path, e);
                process::exit(1);
            }
        }
    } else {
        print_help();
        return;
    };

    let mut tools = ToolRegistry::new();
    if let Some(t_path) = tools_file {
        match fs::read_to_string(&t_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(j) => match ToolRegistry::from_mock_json(&j) {
                    Ok(t) => tools = t,
                    Err(e) => {
                        eprintln!("Error loading mock tools: {}", e);
                        process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error parsing mock tools JSON: {}", e);
                    process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("Error reading tools file '{}': {}", t_path, e);
                process::exit(1);
            }
        }
    }

    let grants = grants_str.map(|s| {
        s.split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
    });

    if check_only {
        match speran::check(&source, Some(&tools), grants.as_deref()) {
            Ok(_) => {
                if json_output {
                    println!("{{\"ok\": true, \"message\": \"Check passed\"}}");
                } else {
                    println!("Syntax and static capability checks passed.");
                }
            }
            Err(mut err) => {
                err.attach_source(&source);
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&err.to_json()).unwrap());
                } else {
                    print_error(&err);
                }
                process::exit(1);
            }
        }
        return;
    }

    let mut limits = Limits::default();
    if let Some(ms) = max_steps {
        limits.max_steps = ms;
    }
    if let Some(mt) = max_tool_calls {
        limits.max_tool_calls = mt;
    }
    if let Some(to) = timeout_ms {
        limits.timeout_ms = Some(to);
    }

    let mut runner = SperanRunner::new()
        .with_tools(tools)
        .with_limits(limits);

    if let Some(g) = grants {
        runner = runner.with_grants(g);
    }

    let report = runner.execute(&source);
    handle_report(&report, json_output);
}

fn handle_report(report: &RunReport, json_output: bool) {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report.to_json()).unwrap());
    } else {
        if !report.emits.is_empty() {
            println!("--- Emits ---");
            for e in &report.emits {
                println!("{}", e.display());
            }
            println!("-------------");
        }

        if let Some(err) = &report.error {
            print_error(err);
            process::exit(1);
        } else {
            println!("{}", report.value.repr());
        }
    }
}

fn print_error(err: &SperanError) {
    eprintln!("\x1b[1;31mError [{}]:\x1b[0m {}", err.kind.name(), err.message);
    if let Some(span) = err.span {
        eprintln!("  --> line {}, col {}", span.line, span.col);
    }
    if let Some(code) = &err.code {
        eprintln!("   |");
        eprintln!("   | {}", code);
        eprintln!("   |");
    }
    if let Some(hint) = &err.hint {
        eprintln!("\x1b[1;36m  Hint:\x1b[0m {}", hint);
    }
}

fn run_repl() {
    println!("Speran v{} REPL", env!("CARGO_PKG_VERSION"));
    println!("Type 'exit' to quit, or write Speran expressions and statements.");
    let mut runner = SperanRunner::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("spe> ");
        let _ = stdout.flush();
        let mut line = String::new();
        if stdin.read_line(&mut line).is_err() || line.trim() == "exit" {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let report = runner.execute(trimmed);
        if let Some(err) = &report.error {
            print_error(err);
        } else {
            println!("{}", report.value.repr());
        }
    }
}

fn print_help() {
    println!(
        r#"Speran - A safe, token-efficient language for LLM agents.

USAGE:
    speran <file.spe> [OPTIONS]
    speran -e "<code>" [OPTIONS]
    speran --repl

OPTIONS:
    -e, --eval <code>         Execute code string directly
    --check                   Run parser and static capability checker without executing
    --tools <file.json>       Load mock tool definitions from a JSON file
    --grants <list>           Comma-separated capabilities to grant (e.g. github.*,slack.post)
    --json                    Output structured JSON report (value, trace, usage, error)
    --max-steps <N>           Maximum execution steps allowed (default: 100000)
    --max-tools <N>           Maximum tool calls allowed (default: 50)
    --timeout <ms>            Timeout in milliseconds (default: 30000)
    --repl                    Start interactive REPL session
    -h, --help                Show this help message
    -v, --version             Show version
"#
    );
}

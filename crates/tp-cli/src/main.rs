use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "pax",
    version = pax_core::build_info::VERSION_STRING,
    about = "Terminal Session Manager — Tilix-like workspace with heterogeneous panels",
    long_about = "Pax is a GUI workspace manager with split/tab panels.\n\n\
        Run without arguments to open a new empty workspace.\n\
        Use Ctrl+Shift+H/J/T to add splits and tabs, Ctrl+S to save.\n\n\
        SHORTCUTS:\n  \
        Ctrl+Shift+H    Horizontal split (new terminal right)\n  \
        Ctrl+Shift+J    Vertical split (new terminal below)\n  \
        Ctrl+Shift+T    Add tab\n  \
        Ctrl+Shift+W    Close panel\n  \
        Ctrl+N/P        Focus next/previous panel\n  \
        Ctrl+S          Save workspace to file\n  \
        Ctrl+Q          Quit"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Directory for log file (default: current directory)
    #[arg(long, global = true)]
    log_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a new empty workspace (default if no command given)
    New {
        /// Workspace name
        #[arg(short, long, default_value = "untitled")]
        name: String,
        /// Save to this file on Ctrl+S
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Launch a workspace from a JSON config file
    Launch {
        /// Path to workspace JSON config
        config: PathBuf,
    },
    /// List known workspaces
    List,
    /// Search command history and saved output
    Search {
        /// Search query (FTS5 syntax)
        query: String,
        /// Max results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Export a template workspace config
    Init {
        /// Output file path
        #[arg(default_value = "workspace.json")]
        output: PathBuf,
        /// Template: simple, grid
        #[arg(short, long, default_value = "simple")]
        template: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Log directory: --log-dir flag, or current directory
    let log_dir = cli
        .log_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("pax.log"))
        .unwrap_or_else(|_| std::fs::File::create("/tmp/pax.log").unwrap());

    tracing_subscriber::fmt()
        .with_env_filter("pax=debug,pax_gui=debug,pax_core=info")
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    // Panic handler: log to file + stderr before crashing
    {
        let crash_log = log_dir.join("pax.log");
        std::panic::set_hook(Box::new(move |info| {
            let msg = format!(
                "\n=== PAX CRASH {} ===\n{}\nBacktrace:\n{}\n",
                "timestamp",
                info,
                std::backtrace::Backtrace::force_capture()
            );
            eprintln!("{}", msg);
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&crash_log) {
                use std::io::Write;
                let _ = f.write_all(msg.as_bytes());
            }
        }));
    }

    match cli.command {
        // No subcommand → show welcome screen
        None => {
            pax_gui::app::run_app(None, None)?;
        }
        // Explicit new → open empty workspace directly
        Some(Commands::New { name, output }) => {
            let ws = pax_core::template::empty_workspace(&name);
            pax_gui::app::run_app(Some(ws), output.as_deref())?;
        }
        Some(Commands::Launch { config }) => {
            let ws = pax_core::config::load_workspace(&config)
                .with_context(|| format!("Failed to load {}", config.display()))?;

            pax_gui::app::run_app(Some(ws), Some(&config))?;
        }
        Some(Commands::List) => {
            let db_path = pax_db::Database::default_path();
            let db = pax_db::Database::open(&db_path)?;
            let workspaces = db.list_workspaces()?;

            if workspaces.is_empty() {
                println!("No workspaces found. Use 'pax launch <config.json>' to start.");
            } else {
                println!(
                    "{:<20} {:<40} {:<20} {}",
                    "Name", "Config", "Last Opened", "Opens"
                );
                println!("{}", "-".repeat(90));
                for ws in workspaces {
                    println!(
                        "{:<20} {:<40} {:<20} {}",
                        ws.name,
                        ws.config_path.unwrap_or_default(),
                        ws.last_opened,
                        ws.open_count
                    );
                }
            }
        }
        Some(Commands::Search { query, limit }) => {
            let db_path = pax_db::Database::default_path();
            let db = pax_db::Database::open(&db_path)?;

            println!("=== Commands ===");
            let cmds = db.search_commands(&query, limit)?;
            if cmds.is_empty() {
                println!("  (no results)");
            }
            for cmd in cmds {
                println!(
                    "  [{}] {} | {} | {}",
                    cmd.executed_at,
                    cmd.workspace_name.unwrap_or_default(),
                    cmd.panel_id.unwrap_or_default(),
                    cmd.command
                );
            }

            println!("\n=== Output ===");
            let outputs = db.search_output(&query, limit)?;
            if outputs.is_empty() {
                println!("  (no results)");
            }
            for out in outputs {
                println!(
                    "  [{}] {} | {} | {}...",
                    out.saved_at,
                    out.workspace_name.unwrap_or_default(),
                    out.panel_id,
                    &out.content[..out.content.len().min(80)]
                );
            }
        }
        Some(Commands::Init { output, template }) => {
            let ws = match template.as_str() {
                "grid" => pax_core::template::grid_2x2("my-workspace"),
                _ => pax_core::template::simple_hsplit("my-workspace", 2),
            };
            pax_core::config::save_workspace(&ws, &output)?;
            println!("Workspace config written to {}", output.display());
        }
    }

    Ok(())
}

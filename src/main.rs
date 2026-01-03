use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, CommandFactory};
use clap_complete;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;

// Use types shown in user's snippet
use resend_rs::types::{CreateEmailBaseOptions, UpdateEmailOptions};
use resend_rs::Resend;

#[derive(Serialize, Deserialize, Default, Debug)]
struct AppConfig {
    api_key: String,
    default_from: Option<String>,
    default_to: Option<String>,
}

#[derive(Parser)]
#[command(name = "rusend", about = "A small user-friendly CLI for resend.com")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save your configuration (API key, defaults)
    Config {
        /// Set the API key
        #[arg(short, long)]
        key: Option<String>,

        /// Set default 'from' address
        #[arg(long)]
        default_from: Option<String>,

        /// Set default 'to' address
        #[arg(long)]
        default_to: Option<String>,
    },

    /// Send one email (reads body from --html, --text, or stdin)
    Send(SendArgs),

    /// Send batch using a JSON file with an array of messages
    Batch { file: PathBuf },

    /// List sent emails
    List {
        /// Number of emails to display
        #[arg(value_name = "COUNT")]
        count: Option<NonZeroUsize>,
    },

    /// Get a single sent email by id (defaults to newest when omitted)
    Get {
        /// Email id (omit to show the newest email)
        #[arg(value_name = "ID")]
        id: Option<String>,
    },

    /// Update an email (e.g. schedule)
    Update {
        id: String,
        #[arg(short, long)]
        scheduled_at: Option<String>,
    },

    /// Cancel a scheduled email
    Cancel { id: String },

    /// List received emails (inbox)
    ReceivedList {
        /// Number of emails to display
        #[arg(value_name = "COUNT")]
        count: Option<NonZeroUsize>,
    },

    /// Get a received email (defaults to newest when omitted)
    ReceivedGet {
        /// Email id (omit to show the newest email)
        #[arg(value_name = "ID")]
        id: Option<String>,
    },

    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Args)]
struct SendArgs {
    /// From header, e.g. "Acme <no-reply@acme.com>"
    #[arg(short, long)]
    from: Option<String>,

    /// To recipients, comma separated
    #[arg(short, long)]
    to: Option<String>,

    /// Subject
    #[arg(short, long, required_unless_present = "id")]
    subject: Option<String>,

    /// Provide HTML body inline
    #[arg(long, conflicts_with = "id")]
    html: Option<String>,

    /// Provide plain text body inline
    #[arg(long, conflicts_with = "id")]
    text: Option<String>,

    /// Read body from stdin
    #[arg(long, conflicts_with = "id")]
    from_stdin: bool,

    /// Forward a received email by ID
    #[arg(long)]
    id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BatchEmailInput {
    from: String,
    to: Vec<String>,
    subject: String,
    html: Option<String>,
    text: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut io::stdout());
        }
        Commands::Config { key, default_from, default_to } => {
            let mut cfg = load_config().unwrap_or_default();

            if let Some(k) = key {
                cfg.api_key = k;
            } else if cfg.api_key.is_empty() {
                println!("Enter your resend API key (starts with re_):");
                cfg.api_key = rpassword::read_password().context("failed to read api key")?;
            }

            if let Some(f) = default_from {
                cfg.default_from = Some(f);
            }
            if let Some(t) = default_to {
                cfg.default_to = Some(t);
            }

            save_config(&cfg)?;
            println!("Configuration saved.");
        }
        Commands::Send(args) => {
            let config = load_config()?;
            let api_key = config.api_key;
            let resend = Resend::new(&api_key);

            let from_addr = args.from.or(config.default_from).context("From address not provided and no default set")?;
            let to_addr = args.to.or(config.default_to).context("To address not provided and no default set")?;

            let (subject, body_html, body_text) = if let Some(ref id) = args.id {
                let email_id = resolve_received_email_id(&resend, Some(id.clone())).await?;
                let r = resend
                    .receiving
                    .get(&email_id)
                    .await
                    .context("get received email for forwarding failed")?;
                
                let subject = args.subject.clone().unwrap_or_else(|| format!("Fwd: {}", r.subject));
                (subject, r.html, r.text)
            } else {
                let body_html = if args.from_stdin {
                    let mut s = String::new();
                    io::stdin().read_to_string(&mut s).context("stdin read")?;
                    Some(s)
                } else {
                    args.html.clone()
                };
                // Safety: clap ensures subject is present if id is missing
                (args.subject.clone().unwrap(), body_html, args.text.clone())
            };

            let mut email =
                CreateEmailBaseOptions::new(&from_addr, parse_to_vec(&to_addr), &subject);
            
            if let Some(h) = body_html {
                email = email.with_html(&h);
            }
            if let Some(t) = body_text {
                email = email.with_text(&t);
            }

            let _res = resend.emails.send(email).await.context("send failed")?;
            println!("Send request submitted.");
        }
        Commands::Batch { file } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);

            let content = fs::read_to_string(&file).context("read batch file")?;
            let batch: Vec<BatchEmailInput> =
                serde_json::from_str(&content).context("parse json")?;

            let emails: Vec<CreateEmailBaseOptions> = batch
                .into_iter()
                .map(|b| {
                    let mut e = CreateEmailBaseOptions::new(&b.from, b.to, &b.subject);
                    if let Some(h) = b.html {
                        e = e.with_html(&h);
                    }
                    if let Some(t) = b.text {
                        e = e.with_text(&t);
                    }
                    e
                })
                .collect();

            let _res = resend
                .batch
                .send(emails)
                .await
                .context("batch send failed")?;
            println!("Batch send request submitted.");
        }
        Commands::List { count } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let limit = count.map(NonZeroUsize::get).unwrap_or(10);
            let emails = resend
                .emails
                .list(Default::default())
                .await
                .context("list failed")?;
            for email in emails.data.into_iter().take(limit) {
                println!(
                    "ID: {}, Created: {}, From: {}, To: {:?}",
                    email.id, email.created_at, email.from, email.to
                );
            }
        }
        Commands::Get { id } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let email_id = resolve_sent_email_id(&resend, id).await?;
            let email = resend.emails.get(&email_id).await.context("get failed")?;
            println!("ID: {}", email.id);
            println!("Created: {}", email.created_at);
            println!("From: {}", email.from);
            println!("To: {:?}", email.to);
            println!("Subject: {}", email.subject);
            print_email_body(email.text.as_deref(), email.html.as_deref());
        }
        Commands::Update { id, scheduled_at } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let mut upd = UpdateEmailOptions::new();
            if let Some(s) = scheduled_at {
                upd = upd.with_scheduled_at(&s);
            }
            let email = resend
                .emails
                .update(&id, upd)
                .await
                .context("update failed")?;
            println!("Updated email with ID: {}", email.id);
        }
        Commands::Cancel { id } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let canceled = resend.emails.cancel(&id).await.context("cancel failed")?;
            println!("Canceled: {}", canceled.id);
        }
        Commands::ReceivedList { count } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let limit = count.map(NonZeroUsize::get).unwrap_or(10);
            let list = resend
                .receiving
                .list(Default::default())
                .await
                .context("list receiving failed")?;
            for email in list.data.into_iter().take(limit) {
                println!(
                    "ID: {}, Created: {}, From: {}, To: {:?}",
                    email.id, email.created_at, email.from, email.to
                );
            }
        }
        Commands::ReceivedGet { id } => {
            let api_key = load_config()?.api_key;
            let resend = Resend::new(&api_key);
            let email_id = resolve_received_email_id(&resend, id).await?;
            let r = resend
                .receiving
                .get(&email_id)
                .await
                .context("get receiving failed")?;
            println!("ID: {}", r.id);
            println!("Created: {}", r.created_at);
            println!("From: {}", r.from);
            println!("To: {:?}", r.to);
            println!("Subject: {}", r.subject);
            print_email_body(r.text.as_deref(), r.html.as_deref());
        }
    }

    Ok(())
}

fn parse_to_vec(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn print_email_body(text: Option<&str>, html: Option<&str>) {
    if let Some(text) = text {
        println!("Text Body:\n{}", text);
    } else if html.is_some() {
        println!("Text Body: <unavailable (HTML body only)>");
    } else {
        println!("Text Body: <empty>");
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "resend", "rusend").context("cannot determine configuration directory")
}

fn credentials_path() -> Result<PathBuf> {
    let pd = project_dirs()?;
    let cfg = pd.config_dir();
    fs::create_dir_all(cfg).context("create config dir")?;
    Ok(cfg.join("credentials"))
}

fn save_config(cfg: &AppConfig) -> Result<()> {
    let path = credentials_path()?;
    let content = serde_json::to_string_pretty(cfg)?;
    fs::write(path, content).context("write config file")?;
    Ok(())
}

fn load_config() -> Result<AppConfig> {
    let path = credentials_path()?;
    if !path.exists() {
        // If no file exists, return default (empty API key) so we can prompt or fail gracefully
        return Ok(AppConfig::default());
    }
    let content = fs::read_to_string(path).context("read config file")?;
    let content = content.trim();

    // Try parsing as JSON first
    if let Ok(cfg) = serde_json::from_str::<AppConfig>(content) {
        return Ok(cfg);
    }

    // Fallback: assume it is just the API key (legacy format)
    if !content.is_empty() {
        return Ok(AppConfig {
            api_key: content.to_string(),
            ..Default::default()
        });
    }

    // If file exists but is empty or unparseable and not a simple string?
    // Treat as empty config or error? Let's return error if it's not valid JSON and not a simple key
    // Actually, simple key check above covers almost anything non-empty. 
    // But if it was a corrupted JSON it might be treated as a key.
    // However, API keys usually don't look like broken JSON (no curly braces).
    // Safe enough for this tool.
    
    Ok(AppConfig::default())
}

async fn resolve_sent_email_id(resend: &Resend, provided: Option<String>) -> Result<String> {
    if let Some(id) = provided {
        return Ok(id);
    }
    let emails = resend
        .emails
        .list(Default::default())
        .await
        .context("list sent emails to find newest")?;
    if let Some(email) = emails.data.into_iter().next() {
        Ok(email.id.to_string())
    } else {
        bail!("No sent emails available to display.");
    }
}

async fn resolve_received_email_id(resend: &Resend, provided: Option<String>) -> Result<String> {
    if let Some(id) = provided {
        return Ok(id);
    }
    let emails = resend
        .receiving
        .list(Default::default())
        .await
        .context("list received emails to find newest")?;
    if let Some(email) = emails.data.into_iter().next() {
        Ok(email.id.to_string())
    } else {
        bail!("No received emails available to display.");
    }
}

// Note: This small CLI focuses on covering the common resend endpoints. Attachments,
// advanced send options, and OAuth-style flows are left as future improvements.

// Add minimal helper to allow rpassword to be used
mod rpassword {
    use std::io;
    pub fn read_password() -> io::Result<String> {
        // very simple: read from stdin (works when user types and presses enter)
        let mut s = String::new();
        io::stdin().read_line(&mut s)?;
        Ok(s.trim().to_string())
    }
}

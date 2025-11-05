use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;

// Use types shown in user's snippet
use resend_rs::types::{CreateEmailBaseOptions, UpdateEmailOptions};
use resend_rs::Resend;

#[derive(Parser)]
#[command(name = "rusend", about = "A small user-friendly CLI for resend.com")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save your API key for reuse
    Config {
        /// Set the API key (if omitted, will prompt)
        #[arg(short, long)]
        key: Option<String>,
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

    /// Get a single sent email by id
    Get { id: String },

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

    /// Get a received email
    ReceivedGet { id: String },
}

#[derive(Args)]
struct SendArgs {
    /// From header, e.g. "Acme <no-reply@acme.com>"
    #[arg(short, long)]
    from: String,

    /// To recipients, comma separated
    #[arg(short, long)]
    to: String,

    /// Subject
    #[arg(short, long)]
    subject: String,

    /// Provide HTML body inline
    #[arg(long)]
    html: Option<String>,

    /// Provide plain text body inline
    #[arg(long)]
    text: Option<String>,

    /// Read body from stdin
    #[arg(long)]
    from_stdin: bool,
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
        Commands::Config { key } => {
            let k = match key {
                Some(k) => k,
                None => {
                    println!("Enter your resend API key (starts with re_):");
                    rpassword::read_password().context("failed to read api key")?
                }
            };
            save_api_key(&k)?;
            println!("API key saved.");
        }
        Commands::Send(args) => {
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);

            let body_html = if args.from_stdin {
                let mut s = String::new();
                io::stdin().read_to_string(&mut s).context("stdin read")?;
                Some(s)
            } else {
                args.html.clone()
            };

            let mut email =
                CreateEmailBaseOptions::new(&args.from, parse_to_vec(&args.to), &args.subject);
            if let Some(h) = body_html {
                email = email.with_html(&h);
            } else if let Some(t) = args.text.clone() {
                email = email.with_text(&t);
            }

            let _res = resend.emails.send(email).await.context("send failed")?;
            println!("Send request submitted.");
        }
        Commands::Batch { file } => {
            let api_key = load_api_key()?;
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
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);
            let limit = count.map(NonZeroUsize::get).unwrap_or(20);
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
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);
            let email = resend.emails.get(&id).await.context("get failed")?;
            println!(
                "ID: {}, Created: {}, From: {}, To: {:?}",
                email.id, email.created_at, email.from, email.to
            );
        }
        Commands::Update { id, scheduled_at } => {
            let api_key = load_api_key()?;
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
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);
            let canceled = resend.emails.cancel(&id).await.context("cancel failed")?;
            println!("Canceled: {}", canceled.id);
        }
        Commands::ReceivedList { count } => {
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);
            let limit = count.map(NonZeroUsize::get).unwrap_or(20);
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
            let api_key = load_api_key()?;
            let resend = Resend::new(&api_key);
            let r = resend
                .receiving
                .get(&id)
                .await
                .context("get receiving failed")?;
            println!(
                "ID: {}, Created: {}, From: {}, To: {:?}",
                r.id, r.created_at, r.from, r.to
            );
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

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "resend", "rusend").context("cannot determine configuration directory")
}

fn credentials_path() -> Result<PathBuf> {
    let pd = project_dirs()?;
    let cfg = pd.config_dir();
    fs::create_dir_all(cfg).context("create config dir")?;
    Ok(cfg.join("credentials"))
}

fn save_api_key(key: &str) -> Result<()> {
    let path = credentials_path()?;
    fs::write(path, key).context("write api key")?;
    Ok(())
}

fn load_api_key() -> Result<String> {
    let path = credentials_path()?;
    let key = fs::read_to_string(path).context("read api key (have you run `rusend config`?)")?;
    Ok(key.trim().to_string())
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

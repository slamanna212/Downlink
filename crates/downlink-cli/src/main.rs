//! Headless dev tool: probe a source, list channels, dump the guide, print a (redacted) play plan.
//! It never requests a stream endpoint; `play-plan` only prints what would be played.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use downlink_core::auth::{MemorySecretStore, SecretStore, SecretString};
use downlink_core::catalog::{self, ChannelFilter};
use downlink_core::redact::RedactingMakeWriter;
use downlink_core::source::SourceId;
use downlink_core::source::dispatcharr::{ClientOptions, Credentials, DispatcharrClient};
use downlink_core::store::{ChannelRow, NewSource, Store};
use downlink_core::{AppError, epg, sync};

#[derive(Parser)]
#[command(name = "downlink", about = "Downlink headless dev tool", version)]
struct Cli {
    /// Dispatcharr base URL, e.g. http://localhost:9191
    #[arg(long, env = "DOWNLINK_SOURCE")]
    source: String,
    /// API key (prefer the DOWNLINK_KEY env var so it stays out of shell history)
    #[arg(long, env = "DOWNLINK_KEY", hide_env_values = true)]
    key: String,
    /// Accept a self-signed certificate
    #[arg(long)]
    insecure: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List channels (adult/radio hidden), optionally searching
    Channels {
        #[arg(long)]
        search: Option<String>,
    },
    /// Dump now/next for visible channels
    Epg {
        /// Only this channel (name, number)
        #[arg(long)]
        channel: Option<String>,
    },
    /// Print the redacted play plan for a channel (number, id or name); never fetches the stream
    PlayPlan { channel: String },
}

struct Ctx {
    store: Store,
    source_id: i64,
    client: DispatcharrClient,
    secrets: MemorySecretStore,
}

async fn setup(cli: &Cli) -> Result<(Ctx, Option<downlink_core::source::UserLevel>), AppError> {
    let store = Store::open_memory().await?;
    let secrets = MemorySecretStore::new();
    let row = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "cli".into(),
            base_url: Some(cli.source.clone()),
            secret_ref: Some("cli/api_key".into()),
            username: None,
            tls_insecure: cli.insecure,
            created_at: sync::now_utc(),
        })
        .await?;
    let key = SecretString::new(cli.key.clone());
    secrets.set("cli/api_key", &key)?;
    let client = DispatcharrClient::new(
        SourceId(row.id),
        &cli.source,
        Credentials::ApiKey(key),
        ClientOptions {
            insecure_tls: cli.insecure,
            ..Default::default()
        },
    )?;
    let info = sync::connect_and_record(&client, &store, row.id).await?;
    sync::sync_channels(&client, &store, row.id, info.user_level).await?;
    Ok((
        Ctx {
            store,
            source_id: row.id,
            client,
            secrets,
        },
        info.user_level,
    ))
}

fn line(c: &ChannelRow) -> String {
    let n = c.number.map_or_else(|| "-".to_owned(), |n| format!("{n}"));
    format!(
        "{n:>6}  {}{}",
        c.name,
        c.quality
            .as_deref()
            .map(|q| format!(" [{q}]"))
            .unwrap_or_default()
    )
}

async fn find(ctx: &Ctx, q: &str) -> Result<ChannelRow, AppError> {
    let f = ChannelFilter {
        source_id: Some(ctx.source_id),
        search: Some(q.to_owned()),
        ..Default::default()
    };
    catalog::list_channels(&ctx.store, &f, 1, 0)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::local(format!("No channel matches '{q}'.")))
}

async fn run(cli: Cli) -> Result<(), AppError> {
    let (ctx, _) = setup(&cli).await?;
    match &cli.cmd {
        Cmd::Channels { search } => {
            let f = ChannelFilter {
                source_id: Some(ctx.source_id),
                search: search.clone(),
                ..Default::default()
            };
            for c in catalog::list_channels(&ctx.store, &f, 10_000, 0).await? {
                println!("{}", line(&c));
            }
        }
        Cmd::Epg { channel } => {
            let now = sync::now_utc();
            epg::sync_guide(
                &ctx.client,
                &ctx.store,
                ctx.source_id,
                epg::visible_window(now),
            )
            .await?;
            let chans = match channel {
                Some(q) => vec![find(&ctx, q).await?],
                None => {
                    let f = ChannelFilter {
                        source_id: Some(ctx.source_id),
                        ..Default::default()
                    };
                    catalog::list_channels(&ctx.store, &f, 10_000, 0).await?
                }
            };
            for c in chans {
                let nn = epg::now_next(&ctx.store, c.id, now).await?;
                let t = |p: &Option<downlink_core::store::ProgrammeRow>| {
                    p.as_ref().map_or("-".to_owned(), |p| {
                        format!(
                            "{} ({}..{} UTC)",
                            p.title.as_deref().unwrap_or("?"),
                            p.start_utc,
                            p.end_utc
                        )
                    })
                };
                println!(
                    "{}\n    now:  {}\n    next: {}",
                    line(&c),
                    t(&nn.now),
                    t(&nn.next)
                );
            }
        }
        Cmd::PlayPlan { channel } => {
            let c = find(&ctx, channel).await?;
            let plan = catalog::resolve_play_plan(
                &ctx.store,
                &downlink_core::auth::ApiKeyAuth(&ctx.secrets),
                c.id,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&plan.redacted())?);
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(RedactingMakeWriter::new(std::io::stderr))
        .init();
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

use std::sync::Arc;
use std::time::Duration;

use rules::pieces::Catalog;
use server::config::Config;
use server::hub::{Hub, HubConfig, random_seed};
use server::store::StoreWriter;
use server::ws;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let catalog = match Catalog::builtin() {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let hub = Hub::new(
        catalog,
        HubConfig {
            ai_min_delay: Duration::from_millis(config.ai_delay_ms),
            max_connections: config.max_connections,
            log: true,
            ..HubConfig::default()
        },
        random_seed(),
    );
    let store = match StoreWriter::spawn(&config.db_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("{} を開けません: {error}", config.db_path);
            std::process::exit(1);
        }
    };
    hub.set_record_sink(Box::new(store.sink()));
    let listener = match TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("{} で待ち受けできません: {error}", config.bind);
            std::process::exit(1);
        }
    };
    eprintln!(
        "待ち受け: ws://{}/ws（許可するオリジン: {}）",
        config.bind,
        config.allowed_origins.join(", ")
    );
    let _ticker = hub.spawn_ticker();
    let hub_for_shutdown = Arc::clone(&hub);
    let shutdown = async move {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("終了します");
        // 開いている接続に、閉じる合図を送る（残っていると、終了が待たされる）
        hub_for_shutdown.shutdown();
    };
    if let Err(error) = ws::serve(listener, hub, &config, shutdown).await {
        eprintln!("サーバーの異常終了: {error}");
        store.finish();
        std::process::exit(1);
    }
    // 頼まれた記録を、保存し終えてから終わる
    store.finish();
}

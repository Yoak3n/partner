use std::sync::OnceLock;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use ai_partner_core::{Agent, OpenAIAdapter, Runtime};
use ai_partner_shared::{AgentEvent, AppConfig, Storage};

/// 全局事件接收器，subscription 通过它接收 runtime 事件
static EVENT_RX: OnceLock<Mutex<mpsc::UnboundedReceiver<AgentEvent>>> = OnceLock::new();

/// 初始化 runtime 并返回 (cmd_tx, event_rx)
/// cmd_tx 用于向 runtime 发送用户输入
pub fn init_runtime() -> mpsc::UnboundedSender<String> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<String>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    EVENT_RX.get_or_init(|| Mutex::new(event_rx));

    let app_config = AppConfig::load().unwrap_or_default();
    let storage = Storage::new().expect("Failed to init storage");

    let adapter = OpenAIAdapter::new();
    let agent = Agent::new(adapter);
    let runtime = Runtime::new(agent, app_config, storage, event_tx);

    tokio::spawn(runtime_loop(runtime, cmd_rx));

    cmd_tx
}

/// 从全局 EVENT_RX 接收一个事件（供 iced subscription 使用）
pub async fn recv_event() -> AgentEvent {
    let rx = EVENT_RX.get().expect("EVENT_RX not initialized");
    let mut guard = rx.lock().await;
    match guard.recv().await {
        Some(evt) => evt,
        None => std::future::pending().await,
    }
}

async fn runtime_loop(mut runtime: Runtime, mut cmd_rx: mpsc::UnboundedReceiver<String>) {
    while let Some(user_input) = cmd_rx.recv().await {
        runtime.send_message(user_input).await;
    }
}

use std::sync::OnceLock;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use ai_partner_core::{Agent, OpenAIAdapter, Runtime};
use ai_partner_shared::{AgentEvent, AppConfig, Storage};

/// 发送给 runtime 的命令
pub enum RuntimeCommand {
    SendMessage(String),
    ListSessions,
    NewSession,
    SwitchSession(String),
    DeleteSession(String),
    PinSession(String),
    ArchiveSession(String),
}

/// 全局事件接收器，subscription 通过它接收 runtime 事件
static EVENT_RX: OnceLock<Mutex<mpsc::UnboundedReceiver<AgentEvent>>> = OnceLock::new();

/// 初始化 runtime 并返回 cmd_tx（在后台线程执行）
pub fn init_runtime() -> mpsc::UnboundedSender<RuntimeCommand> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<RuntimeCommand>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    EVENT_RX.get_or_init(|| Mutex::new(event_rx));

    // 在后台线程初始化 runtime，避免阻塞 UI
    std::thread::spawn(move || {
        let app_config = AppConfig::load().expect("无法加载配置文件");
        let storage = Storage::new().expect("Failed to init storage");
        let adapter = OpenAIAdapter::new();
        let agent = Agent::new(adapter);
        let mut runtime = Runtime::new(agent, app_config.clone(), storage, event_tx);

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(runtime.set_workspace(app_config.workspace));
        rt.block_on(runtime_loop(runtime, cmd_rx));
    });

    // 立即加载 session 列表
    let _ = cmd_tx.send(RuntimeCommand::ListSessions);

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

async fn runtime_loop(mut runtime: Runtime, mut cmd_rx: mpsc::UnboundedReceiver<RuntimeCommand>) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            RuntimeCommand::SendMessage(text) => {
                runtime.send_message(text).await;
            }
            RuntimeCommand::ListSessions => {
                runtime.list_sessions();
            }
            RuntimeCommand::NewSession => {
                runtime.new_session();
            }
            RuntimeCommand::SwitchSession(id) => {
                runtime.switch_session(&id);
            }
            RuntimeCommand::DeleteSession(id) => {
                runtime.delete_session(&id);
            }
            RuntimeCommand::PinSession(id) => {
                runtime.toggle_pin_session(&id);
            }
            RuntimeCommand::ArchiveSession(id) => {
                runtime.toggle_archive_session(&id);
            }
        }
    }
}

use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{clipboard, Color, Element, Length, Subscription, Task, Theme};
use std::time::Duration;
use tokio::sync::mpsc;

use std::collections::HashSet;
use ai_partner_shared::{AgentEvent, Message, SessionSummary};

use crate::components::{message_view, title_bar};
use crate::runtime_bridge::{self, RuntimeCommand};
use crate::tray::TrayEvent;

pub struct App {
    input: String,
    messages: Vec<Message>,
    display_items: Vec<message_view::DisplayItem>,
    is_thinking: bool,
    streaming_content: String,
    copy_toast: Option<String>,
    session_id: Option<String>,
    loading_session_id: Option<String>,
    sessions: Vec<SessionSummary>,
    show_sidebar: bool,
    cmd_tx: mpsc::UnboundedSender<RuntimeCommand>,
    expanded_tool_calls: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    InputChanged(String),
    SendPressed,
    RuntimeEvent(AgentEvent),
    TrayEvent(TrayEvent),
    WindowClose,
    WindowMaximizeToggle,
    WindowMinimize,
    WindowDrag,
    CopyText(String),
    ClearCopyToast,
    ToggleToolCall(String),
    ToggleSidebar,
    NewSession,
    SwitchSession(String),
    DeleteSession(String),
    PinSession(String),
    ArchiveSession(String),
    Noop,
}

fn with_latest_window(task_fn: fn(iced::window::Id) -> Task<AppEvent>) -> Task<AppEvent> {
    iced::window::latest().then(move |id| {
        if let Some(id) = id {
            task_fn(id)
        } else {
            Task::none()
        }
    })
}

impl App {
    pub fn new() -> (Self, Task<AppEvent>) {
        let cmd_tx = runtime_bridge::init_runtime();

        let cmd_tx_clone = cmd_tx.clone();
        let setup = Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                crate::apply_rounded_corners();
                // 启动时加载 session 列表
                let _ = cmd_tx_clone.send(RuntimeCommand::ListSessions);
            },
            |_| AppEvent::Noop,
        );

        (
            Self {
                input: String::new(),
                messages: Vec::new(),
                display_items: Vec::new(),
                is_thinking: false,
                streaming_content: String::new(),
                copy_toast: None,
                session_id: None,
                loading_session_id: None,
                sessions: Vec::new(),
                show_sidebar: true,
                cmd_tx,
                expanded_tool_calls: HashSet::new(),
            },
            setup,
        )
    }

    fn refresh_display_items(&mut self) {
        self.display_items = message_view::merge_messages(&self.messages);
    }

    fn handle_input(&mut self, event: AppEvent) -> Task<AppEvent> {
        match event {
            AppEvent::InputChanged(value) => {
                self.input = value;
                Task::none()
            }
            AppEvent::SendPressed => {
                if self.input.is_empty() {
                    return Task::none();
                }
                let _ = self.cmd_tx.send(RuntimeCommand::SendMessage(self.input.clone()));
                self.messages.push(Message::user(&self.input));
                self.refresh_display_items();
                self.input.clear();
                self.is_thinking = true;
                let _ = self.cmd_tx.send(RuntimeCommand::ListSessions);
                Task::none()
            }
            _ => unreachable!(),
        }
    }

    fn handle_runtime(&mut self, agent_event: AgentEvent) -> Task<AppEvent> {
        match agent_event {
            AgentEvent::Thinking => {
                self.is_thinking = true;
                self.streaming_content.clear();
            }
            AgentEvent::Delta(chunk) => {
                self.streaming_content.push_str(&chunk);
            }
            AgentEvent::MessageComplete(msg) => {
                let has_tool_calls = msg.tool_calls.is_some();
                self.messages.push(msg);
                self.refresh_display_items();
                if !has_tool_calls {
                    self.is_thinking = false;
                    self.streaming_content.clear();
                }
            }
            AgentEvent::Error(err) => {
                self.messages.push(Message::system(format!("[Error] {err}")));
                self.refresh_display_items();
                self.is_thinking = false;
                self.streaming_content.clear();
            }
            AgentEvent::ToolCallResult { call_id, result } => {
                self.messages.push(Message::tool_result(&call_id, &result));
                self.refresh_display_items();
            }
            AgentEvent::Done => {
                self.is_thinking = false;
            }
            AgentEvent::SessionsLoaded(list) => {
                self.sessions = list;
            }
            AgentEvent::SessionCreated(id) => {
                self.session_id = Some(id);
                self.messages.clear();
                self.refresh_display_items();
            }
            AgentEvent::SessionSwitched(id) => {
                self.session_id = Some(id);
                // 不立即关闭侧边栏，等消息加载完成后再关闭
                // 不清空消息，保持当前显示直到新消息加载完成
            }
            AgentEvent::MessagesLoaded { messages, .. } => {
                self.messages = messages;
                self.refresh_display_items();
                self.loading_session_id = None;
                // 消息加载完成后再关闭侧边栏
                self.show_sidebar = false;
            }
            _ => {}
        }
        Task::none()
    }

    fn handle_window(event: AppEvent) -> Task<AppEvent> {
        match event {
            AppEvent::WindowClose => {
                #[cfg(windows)]
                {
                    // Windows: 隐藏窗口到托盘，而不是关闭
                    with_latest_window(|id| iced::window::set_mode(id, iced::window::Mode::Hidden))
                }
                #[cfg(not(windows))]
                {
                    // Linux (niri/Wayland): 直接关闭窗口
                    with_latest_window(|id| iced::window::close(id))
                }
            }
            AppEvent::WindowMaximizeToggle => {
                with_latest_window(|id| iced::window::toggle_maximize(id))
            }
            AppEvent::WindowMinimize => {
                with_latest_window(|id| iced::window::minimize(id, true))
            }
            AppEvent::WindowDrag => with_latest_window(|id| iced::window::drag(id)),
            _ => unreachable!(),
        }
    }

    fn handle_tray(event: TrayEvent) -> Task<AppEvent> {
        match event {
            TrayEvent::Show => with_latest_window(|id| {
                iced::window::set_mode(id, iced::window::Mode::Windowed)
                    .chain(iced::window::gain_focus(id))
            }),
            TrayEvent::Quit => iced::exit(),
        }
    }

    fn handle_clipboard(&mut self, event: AppEvent) -> Task<AppEvent> {
        match event {
            AppEvent::CopyText(t) => {
                self.copy_toast = Some(t.clone());
                clipboard::write(t).chain(Task::perform(
                    async { tokio::time::sleep(Duration::from_millis(1500)).await },
                    |_| AppEvent::ClearCopyToast,
                ))
            }
            AppEvent::ClearCopyToast => {
                self.copy_toast = None;
                Task::none()
            }
            _ => unreachable!(),
        }
    }

    fn handle_sidebar(&mut self, event: AppEvent) -> Task<AppEvent> {
        match event {
            AppEvent::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
                if self.show_sidebar {
                    let _ = self.cmd_tx.send(RuntimeCommand::ListSessions);
                }
                Task::none()
            }
            AppEvent::NewSession => {
                let _ = self.cmd_tx.send(RuntimeCommand::NewSession);
                Task::none()
            }
            AppEvent::SwitchSession(id) => {
                self.loading_session_id = Some(id.clone());
                let _ = self.cmd_tx.send(RuntimeCommand::SwitchSession(id));
                Task::none()
            }
            AppEvent::DeleteSession(id) => {
                let _ = self.cmd_tx.send(RuntimeCommand::DeleteSession(id));
                Task::none()
            }
            AppEvent::PinSession(id) => {
                let _ = self.cmd_tx.send(RuntimeCommand::PinSession(id));
                Task::none()
            }
            AppEvent::ArchiveSession(id) => {
                let _ = self.cmd_tx.send(RuntimeCommand::ArchiveSession(id));
                Task::none()
            }
            _ => unreachable!(),
        }
    }

    pub fn update(&mut self, event: AppEvent) -> Task<AppEvent> {
        match &event {
            AppEvent::InputChanged(_) | AppEvent::SendPressed => self.handle_input(event),
            AppEvent::RuntimeEvent(e) => self.handle_runtime(e.clone()),
            AppEvent::WindowClose | AppEvent::WindowMaximizeToggle | AppEvent::WindowMinimize | AppEvent::WindowDrag => {
                Self::handle_window(event)
            }
            AppEvent::TrayEvent(e) => Self::handle_tray(e.clone()),
            AppEvent::CopyText(_) | AppEvent::ClearCopyToast => self.handle_clipboard(event),
            AppEvent::ToggleToolCall(id) => {
                if !self.expanded_tool_calls.remove(id) {
                    self.expanded_tool_calls.insert(id.clone());
                }
                Task::none()
            }
            AppEvent::ToggleSidebar | AppEvent::NewSession | AppEvent::SwitchSession(_) | AppEvent::DeleteSession(_) | AppEvent::PinSession(_) | AppEvent::ArchiveSession(_) => {
                self.handle_sidebar(event)
            }
            AppEvent::Noop => Task::none(),
        }
    }

    pub fn view(&self) -> Element<'_, AppEvent> {
        // Sidebar
        let sidebar = if self.show_sidebar {
            let pinned: Vec<_> = self.sessions.iter().filter(|s| s.pinned && !s.archived).collect();
            let active: Vec<_> = self.sessions.iter().filter(|s| !s.pinned && !s.archived).collect();
            let archived: Vec<_> = self.sessions.iter().filter(|s| s.archived).collect();

            let mut sidebar_items = column![].spacing(0);

            sidebar_items = sidebar_items.push(
                button(text("＋ 新建会话").size(14))
                    .on_press(AppEvent::NewSession)
                    .width(Length::Fill)
                    .padding(iced::Padding::from([10, 12])),
            );

            sidebar_items = sidebar_items.push(
                container(column![])
                    .height(1)
                    .width(Length::Fill)
                    .style(|_| iced::widget::container::Style {
                        background: Some(Color::from_rgb(0.3, 0.3, 0.3).into()),
                        ..Default::default()
                    }),
            );

            // 置顶组
            if !pinned.is_empty() {
                sidebar_items = sidebar_items.push(
                    container(text("📌 置顶").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)))
                        .padding(8.0),
                );
                for session in pinned {
                    sidebar_items = sidebar_items.push(self.render_session_item(session));
                }
            }

            // 活跃组
            if !active.is_empty() {
                sidebar_items = sidebar_items.push(
                    container(text("💬 会话").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)))
                        .padding(8.0),
                );
                for session in active {
                    sidebar_items = sidebar_items.push(self.render_session_item(session));
                }
            }

            // 归档组
            if !archived.is_empty() {
                sidebar_items = sidebar_items.push(
                    container(text("📦 归档").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)))
                        .padding(8.0),
                );
                for session in archived {
                    sidebar_items = sidebar_items.push(self.render_session_item(session));
                }
            }

            let sidebar_content = column![
                container(
                    row![
                        text("会话").size(16).color(Color::from_rgb(0.9, 0.9, 0.9)),
                        Space::new().width(Length::Fill),
                        button(text("✕").size(14))
                            .on_press(AppEvent::ToggleSidebar)
                            .padding(4),
                    ]
                )
                .padding(iced::Padding::from([12, 12]))
                .width(Length::Fill),
                scrollable(sidebar_items).height(Length::Fill),
            ]
            .spacing(0);

            container(sidebar_content)
                .width(240)
                .height(Length::Fill)
                .style(|_| iced::widget::container::Style {
                    background: Some(Color::from_rgb(0.12, 0.12, 0.14).into()),
                    ..Default::default()
                })
        } else {
            container(
                button(text("☰").size(18))
                    .on_press(AppEvent::ToggleSidebar)
                    .padding(iced::Padding::from([8, 10])),
            )
            .height(Length::Fill)
            .align_y(iced::Alignment::Start)
            .padding(8.0)
        };

        // Chat area — merge tool calls with their results
        let expanded_set = &self.expanded_tool_calls;
        let messages = self.display_items
            .iter()
            .fold(column![].spacing(8), |col, item| {
                match item {
                    message_view::DisplayItem::Message(msg) => {
                        col.push(message_view::view_message(msg))
                    }
                    message_view::DisplayItem::ToolCall { call, result } => {
                        let is_expanded = expanded_set.contains(&call.id);
                        col.push(message_view::view_tool_call(call, result, is_expanded))
                    }
                }
            });

        let messages = if !self.streaming_content.is_empty() {
            let streaming_label = text("AI").size(11).color(Color::from_rgb(0.6, 0.8, 1.0));
            let streaming_body = text(self.streaming_content.clone()).size(15);
            let streaming_bubble =
                container(column![streaming_label, streaming_body].spacing(4))
                    .padding(12)
                    .max_width(700)
                    .width(Length::Shrink)
                    .style(container::rounded_box);
            let streaming_msg = container(streaming_bubble)
                .width(Length::Fill)
                .padding(iced::Padding::from([0, 16]));
            messages.push(streaming_msg)
        } else {
            messages
        };

        let thinking_indicator = if self.is_thinking && self.streaming_content.is_empty() {
            container(text("Thinking...").size(14))
                .padding(8)
                .width(Length::Fill)
        } else {
            container(column![])
        };

        let input_row = text_input("Type a message...", &self.input)
            .on_input(AppEvent::InputChanged)
            .on_submit(AppEvent::SendPressed)
            .padding(10)
            .size(16);

        let chat_body = column![
            scrollable(messages).height(Length::Fill),
            thinking_indicator,
            input_row,
        ]
        .spacing(8)
        .padding(16);

        // Toast
        let toast = if let Some(ref t) = self.copy_toast {
            let preview: String = t.chars().take(20).collect();
            let preview = if t.chars().count() > 20 {
                format!("{preview}...")
            } else {
                preview
            };
            Some(
                container(
                    text(format!("已复制: {preview}"))
                        .size(13)
                        .color(Color::WHITE),
                )
                .padding(iced::Padding::from([6, 14]))
                .style(|_| iced::widget::container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.75).into()),
                    border: iced::border::rounded(8),
                    ..Default::default()
                }),
            )
        } else {
            None
        };

        // Assemble
        let main_content = container(
            column![title_bar::view(), chat_body]
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::border::rounded(12),
                ..iced::widget::container::Style::default()
            }
        });

        let base = row![sidebar, main_content].height(Length::Fill);

        // toast 作为覆盖层，不影响布局
        if let Some(toast_widget) = toast {
            let toast_overlay = container(toast_widget)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center)
                .padding(8.0);
            iced::widget::stack![base, toast_overlay].into()
        } else {
            base.into()
        }
    }

    fn render_session_item<'a>(&'a self, session: &'a SessionSummary) -> Element<'a, AppEvent> {
        let is_active = self.session_id.as_deref() == Some(&session.id);
        let is_loading = self.loading_session_id.as_deref() == Some(&session.id);
        crate::components::session_item::view(session, is_active, is_loading)
    }

    pub fn subscription(&self) -> Subscription<AppEvent> {
        use futures::stream;
        let mut subs = vec![
            Subscription::run(|| {
                stream::unfold((), |()| async {
                    let evt = runtime_bridge::recv_event().await;
                    Some((AppEvent::RuntimeEvent(evt), ()))
                })
            }),
            Subscription::run(|| {
                stream::unfold((), |()| async {
                    let evt = crate::tray::recv_event().await;
                    Some((AppEvent::TrayEvent(evt), ()))
                })
            }),
        ];
        // Windows: 拦截系统级关闭请求（如 Alt+F4），隐藏到托盘而非退出
        #[cfg(windows)]
        subs.push(iced::window::close_requests().map(|_| AppEvent::WindowClose));
        Subscription::batch(subs)
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }
}

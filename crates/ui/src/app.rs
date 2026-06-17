use iced::widget::{column, container, scrollable, text, text_input};
use iced::{Element, Subscription, Task, Theme};
use tokio::sync::mpsc;

use ai_partner_shared::{AgentEvent, Message};

use crate::components::{message_view, title_bar};
use crate::runtime_bridge;
use crate::tray::TrayEvent;

pub struct App {
    input: String,
    messages: Vec<Message>,
    is_thinking: bool,
    streaming_content: String,
    cmd_tx: mpsc::UnboundedSender<String>,
    window_visible: bool,
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
    Noop,
}

impl App {
    pub fn new() -> (Self, Task<AppEvent>) {
        let cmd_tx = runtime_bridge::init_runtime();

        let setup = Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                crate::apply_rounded_corners();
            },
            |_| AppEvent::Noop,
        );

        (
            Self {
                input: String::new(),
                messages: Vec::new(),
                is_thinking: false,
                streaming_content: String::new(),
                cmd_tx,
                window_visible: true,
            },
            setup,
        )
    }

    pub fn update(&mut self, event: AppEvent) -> Task<AppEvent> {
        match event {
            AppEvent::InputChanged(value) => {
                self.input = value;
                Task::none()
            }
            AppEvent::SendPressed => {
                if self.input.is_empty() {
                    return Task::none();
                }

                let user_msg = Message::user(&self.input);
                self.messages.push(user_msg);
                let _ = self.cmd_tx.send(self.input.clone());
                self.input.clear();
                self.is_thinking = true;
                Task::none()
            }
            AppEvent::RuntimeEvent(agent_event) => {
                match agent_event {
                    AgentEvent::Thinking => {
                        self.is_thinking = true;
                        self.streaming_content.clear();
                    }
                    AgentEvent::Delta(chunk) => {
                        self.streaming_content.push_str(&chunk);
                    }
                    AgentEvent::MessageComplete(msg) => {
                        self.messages.push(msg);
                        self.is_thinking = false;
                        self.streaming_content.clear();
                    }
                    AgentEvent::Error(err) => {
                        self.messages.push(Message::system(format!("[Error] {err}")));
                        self.is_thinking = false;
                        self.streaming_content.clear();
                    }
                    AgentEvent::ToolCallResult { call_id: _, result } => {
                        self.messages.push(Message::system(format!("[Tool] {result}")));
                    }
                    AgentEvent::Done => {
                        self.is_thinking = false;
                    }
                    _ => {}
                }
                Task::none()
            }
            AppEvent::TrayEvent(tray_event) => match tray_event {
                TrayEvent::Show => {
                    if self.window_visible {
                        self.window_visible = false;
                        crate::platform::hide_window("AI Partner");
                        Task::none()
                    } else {
                        self.window_visible = true;
                        crate::platform::bring_to_foreground("AI Partner");
                        Task::none()
                    }
                }
                TrayEvent::Quit => iced::exit(),
            },
            // Hide to tray instead of closing
            AppEvent::WindowClose => {
                self.window_visible = false;
                crate::platform::hide_window("AI Partner");
                Task::none()
            }
            AppEvent::WindowMaximizeToggle => iced::window::latest().then(|id| {
                if let Some(id) = id {
                    iced::window::toggle_maximize(id)
                } else {
                    Task::none()
                }
            }),
            AppEvent::WindowMinimize => iced::window::latest().then(|id| {
                if let Some(id) = id {
                    iced::window::minimize(id, true)
                } else {
                    Task::none()
                }
            }),
            AppEvent::WindowDrag => iced::window::latest().then(|id| {
                if let Some(id) = id {
                    iced::window::drag(id)
                } else {
                    Task::none()
                }
            }),
            AppEvent::Noop => Task::none(),
        }
    }

    pub fn view(&self) -> Element<'_, AppEvent> {
        let messages = self
            .messages
            .iter()
            .fold(column![].spacing(8), |col, msg| {
                col.push(message_view::view(msg))
            });

        let messages = if !self.streaming_content.is_empty() {
            messages.push(message_view::streaming_view(&self.streaming_content))
        } else {
            messages
        };

        let thinking_indicator = if self.is_thinking && self.streaming_content.is_empty() {
            container(text("Thinking...").size(14))
                .padding(8)
                .width(iced::Length::Fill)
        } else {
            container(column![])
        };

        let input_row = text_input("Type a message...", &self.input)
            .on_input(AppEvent::InputChanged)
            .on_submit(AppEvent::SendPressed)
            .padding(10)
            .size(16);

        let body = column![
            scrollable(messages).height(iced::Length::Fill),
            thinking_indicator,
            input_row,
        ]
        .spacing(8)
        .padding(16);

        let root = column![
            title_bar::view(),
            body,
        ];

        container(root)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .style(|theme: &iced::Theme| {
                let palette = theme.extended_palette();
                iced::widget::container::Style {
                    background: Some(palette.background.base.color.into()),
                    border: iced::border::rounded(12),
                    ..iced::widget::container::Style::default()
                }
            })
            .into()
    }

    pub fn subscription(&self) -> Subscription<AppEvent> {
        use futures::stream;
        Subscription::batch([
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
        ])
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }
}

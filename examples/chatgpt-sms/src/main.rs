use std::collections::{HashMap, VecDeque};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::Json as ResponseJson;
use axum::Router;
use axum::routing::post;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{error, info, warn, instrument};

const CHATGPT_MODEL: &str = "gpt-3.5-turbo";
const HISTORY_LIMIT: usize = 10;
const CHATGPT_TEMPERATURE: f32 = 0.7;
const CHATGPT_SYSTEM_PROMPT: &str = "You are replying via SMS, so keep messages short and concise.";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    #[serde(rename = "type")]
    webhook_type: String,
    data: WebhookMessage,
}

#[derive(Debug, Deserialize)]
struct WebhookMessage {
    phone_number: String,
    message_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatGPTCompletionRequest {
    model: &'static str,
    temperature: f32,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatGPTCompletionResponse {
    choices: Vec<ChatGPTCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatGPTCompletionChoice {
    message: ChatMessage,
}

#[derive(Serialize)]
struct SendReplyRequest {
    to: String,
    content: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("OpenAI API error: {0}")]
    OpenAI(String),
    #[error("SMS API error: {0}")]
    Sms(String),
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, AppError>;

#[derive(Clone)]
struct AppState {
    message_history: Arc<Mutex<HashMap<String, VecDeque<ChatMessage>>>>,
    http_client: Client,
    sms_send_url: String,
    openai_key: String
}

impl AppState {
    fn new(sms_send_url: String, openai_key: String) -> Self {
        let http_client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            message_history: Arc::new(Mutex::new(HashMap::new())),
            http_client,
            sms_send_url,
            openai_key
        }
    }

    /// Adds a message to history and returns a snapshot of the current conversation.
    #[instrument(skip(self, message), fields(phone_number = %phone_number))]
    async fn add_message_and_get_history(
        &self,
        phone_number: &str,
        message: ChatMessage,
    ) -> Vec<ChatMessage> {
        let mut history_guard = self.message_history.lock().await;
        let messages = history_guard
            .entry(phone_number.to_string())
            .or_insert_with(|| VecDeque::with_capacity(HISTORY_LIMIT));

        messages.push_back(message);
        Self::trim_history(messages);

        messages.iter().cloned().collect()
    }

    /// Adds a message to existing conversation history.
    #[instrument(skip(self, message), fields(phone_number = %phone_number))]
    async fn add_message(&self, phone_number: &str, message: ChatMessage) {
        let mut history_guard = self.message_history.lock().await;
        if let Some(messages) = history_guard.get_mut(phone_number) {
            messages.push_back(message);
            Self::trim_history(messages);
        }
    }

    /// Get a string message reply from ChatGPT with history snapshot.
    #[instrument(skip(self, messages))]
    async fn get_reply(&self, messages: Vec<ChatMessage>) -> Result<String> {
        let system_message = ChatMessage {
            role: "system".to_string(),
            content: CHATGPT_SYSTEM_PROMPT.to_string(),
        };

        // Create new message set with system prompt.
        let mut all_messages = Vec::with_capacity(messages.len() + 1);
        all_messages.push(system_message);
        all_messages.extend(messages);

        // Create request payload.
        let request_body = ChatGPTCompletionRequest {
            model: CHATGPT_MODEL,
            temperature: CHATGPT_TEMPERATURE,
            messages: all_messages,
        };

        // Send chat completion request with history.
        info!("Sending request to ChatGPT API");

        match self
            .http_client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.openai_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<ChatGPTCompletionResponse>().await {
                        Ok(chat_response) => {
                            if let Some(choice) = chat_response.choices.first() {
                                info!("Successfully received ChatGPT response");
                                Ok(choice.message.content.clone())
                            } else {
                                Err(AppError::OpenAI("No choices in response".to_string()))
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse ChatGPT response: {}", e);
                            Err(AppError::OpenAI(format!("Parse error: {}", e)))
                        }
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    error!("ChatGPT API error: {} - {}", status, error_text);
                    Err(AppError::OpenAI(format!("{}: {}", status, error_text)))
                }
            }
            Err(e) => {
                error!("Failed to call ChatGPT API: {}", e);
                Err(AppError::Network(e))
            }
        }
    }

    /// Send the ChatGPT reply back via SMS API.
    #[instrument(skip(self), fields(phone_number = %phone_number, reply_length = reply.len()))]
    async fn send_reply(&self, phone_number: String, reply: String) -> Result<()> {
        let request_body = SendReplyRequest {
            to: phone_number.clone(),
            content: reply.clone(),
        };

        match self
            .http_client
            .post(&self.sms_send_url)
            .json(&request_body)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    info!("Successfully sent reply to {}", phone_number);
                    Ok(())
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    error!("SMS API error: {} - {}", status, error_text);
                    Err(AppError::Sms(format!("{}: {}", status, error_text)))
                }
            }
            Err(e) => {
                error!("Failed to call SMS API: {}", e);
                Err(AppError::Network(e))
            }
        }
    }

    /// Trims history to stay within limits.
    fn trim_history(messages: &mut VecDeque<ChatMessage>) {
        while messages.len() > HISTORY_LIMIT {
            messages.pop_front();
        }
    }
}

#[instrument(skip(state, payload))]
async fn http_webhook(
    State(state): State<AppState>,
    Json(payload): Json<WebhookPayload>,
) -> std::result::Result<StatusCode, (StatusCode, ResponseJson<ErrorResponse>)> {
    if payload.webhook_type != "incoming" {
        warn!("Received non-incoming webhook type: {}", payload.webhook_type);
        return Err((
            StatusCode::BAD_REQUEST,
            ResponseJson(ErrorResponse {
                error: "Invalid webhook type".to_string(),
            }),
        ));
    }

    let phone_number = payload.data.phone_number;
    let message_content = payload.data.message_content.trim().to_string();

    // Create a new task to send the reply so the response isn't blocked.
    info!("Processing incoming message from {}", phone_number);
    tokio::spawn(async move {
        if let Err(e) = process_message(state, phone_number, message_content).await {
            error!("Failed to process message: {}", e);
        }
    });

    Ok(StatusCode::OK)
}

#[instrument(skip(state))]
async fn process_message(
    state: AppState,
    phone_number: String,
    message_content: String,
) -> Result<()> {
    // Store incoming message and get history.
    let incoming_message = ChatMessage {
        role: "user".to_string(),
        content: message_content,
    };
    let history_snapshot = state
        .add_message_and_get_history(&phone_number, incoming_message)
        .await;

    // Generate reply from ChatGPT.
    let reply = state.get_reply(history_snapshot).await.unwrap_or_else(|e| {
        error!("Failed to get ChatGPT reply: {}", e);
        match e {
            AppError::OpenAI(_) => "Sorry, the AI service is currently unavailable!".to_string(),
            AppError::Network(_) => "Sorry, I couldn't connect to the AI service!".to_string(),
            _ => "Sorry, there was an error processing your message!".to_string(),
        }
    });

    // Store outgoing message.
    let outgoing_message = ChatMessage {
        role: "assistant".to_string(),
        content: reply.clone(),
    };
    state.add_message(&phone_number, outgoing_message).await;

    // Finally, send the reply.
    if let Err(e) = state.send_reply(phone_number, reply).await {
        error!("Failed to send SMS reply: {}", e);
        return Err(e);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into())
        )
        .init();

    let sms_send_url = env::var("SMS_SEND_URL").expect("Missing required SMS_SEND_URL env var!");
    let openai_key = env::var("OPENAI_KEY").expect("Missing required OPENAI_KEY env var!");

    let state = AppState::new(sms_send_url, openai_key);

    let app = Router::new()
        .route("/webhook", post(http_webhook))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:3001").await?;

    info!("Starting HTTP listener @ 127.0.0.1:3001");
    axum::serve(listener, app).await?;

    Ok(())
}
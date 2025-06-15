use axum::extract::State;
use crate::{AppState, http_post_handler, http_modem_handler};
use crate::http::get_modem_json_result;
use crate::modem::types::ModemRequest;
use crate::sms::types::SMSOutgoingMessage;
use crate::http::types::{HttpResponse, SendSmsRequest, SendSmsResponse, FetchSmsRequest, FetchSmsResponse, FetchLatestNumbersRequest, FetchLatestNumbersResponse};

http_post_handler!(
    db_sms,
    FetchSmsRequest,
    FetchSmsResponse,
    |state, payload| {
        let messages = state.sms_manager.borrow_database()
            .get_messages(&payload.phone_number, i64::from(payload.limit), i64::from(payload.offset))
            .await?;
        Ok(FetchSmsResponse { messages })
    }
);

http_post_handler!(
    db_latest_numbers,
    FetchLatestNumbersRequest,
    FetchLatestNumbersResponse,
    |state, payload| {
        let numbers = state.sms_manager.borrow_database()
            .get_latest_numbers(i64::from(payload.limit), i64::from(payload.offset))
            .await?;
        Ok(FetchLatestNumbersResponse { numbers })
    }
);

http_post_handler!(
    sms_send,
    SendSmsRequest,
    SendSmsResponse,
    |state, payload| {
        let outgoing = SMSOutgoingMessage {
            phone_number: payload.to,
            content: payload.content,
        };
        let (message_id, response) = state.sms_manager.send_sms(outgoing).await?;
        Ok(SendSmsResponse { message_id, response })
    }
);

http_modem_handler!(sms_get_network_status, ModemRequest::GetNetworkStatus);
http_modem_handler!(sms_get_signal_strength, ModemRequest::GetSignalStrength);
http_modem_handler!(sms_get_network_operator, ModemRequest::GetNetworkOperator);
http_modem_handler!(sms_get_service_provider, ModemRequest::GetServiceProvider);
http_modem_handler!(sms_get_battery_level, ModemRequest::GetBatteryLevel);
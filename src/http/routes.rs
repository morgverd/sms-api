use axum::extract::State;
use crate::{AppState, http_post_handler, http_modem_handler};
use crate::http::get_modem_json_result;
use crate::modem::types::ModemRequest;
use crate::sms::types::{SMSDeliveryReport, SMSMessage, SMSOutgoingMessage};
use crate::http::types::{HttpResponse, PhoneNumberFetchRequest, GlobalFetchRequest, MessageIdFetchRequest, SendSmsRequest, SendSmsResponse};

http_post_handler!(
    db_sms,
    PhoneNumberFetchRequest,
    Vec<SMSMessage>,
    |state, payload| {
        state.sms_manager.borrow_database()
            .get_messages(&payload.phone_number, payload.limit, payload.offset, payload.reverse)
            .await
    }
);

http_post_handler!(
    db_delivery_reports,
    MessageIdFetchRequest,
    Vec<SMSDeliveryReport>,
    |state, payload| {
        state.sms_manager.borrow_database()
            .get_delivery_reports(payload.message_id, payload.limit, payload.offset, payload.reverse)
            .await
    }
);

http_post_handler!(
    db_latest_numbers,
    Option<GlobalFetchRequest>,
    Vec<String>,
    |state, payload| {
        let (limit, offset, reverse) = match payload {
            Some(req) => (req.limit, req.offset, req.reverse),
            None => (None, None, false),
        };

        state.sms_manager.borrow_database()
            .get_latest_numbers(limit, offset, reverse)
            .await
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
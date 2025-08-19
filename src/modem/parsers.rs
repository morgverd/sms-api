use anyhow::{anyhow, Result};
use crate::modem::types::{GNSSFixStatus, GNSSLocation};

pub fn parse_cmgs_result(response: &str) -> Result<u8> {
    let cmgs_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CMGS:"))
        .ok_or(anyhow!("No CMGS response found in buffer"))?;

    cmgs_line
        .trim()
        .strip_prefix("+CMGS:")
        .ok_or(anyhow!("Malformed CMGS response"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid CMGS message reference number"))
}

pub fn parse_creg_response(response: &str) -> Result<(u8, u8)> {
    let creg_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CREG:"))
        .ok_or(anyhow!("No CREG response found in buffer"))?;

    let data = creg_line
        .trim()
        .strip_prefix("+CREG:")
        .ok_or(anyhow!("Malformed CREG response"))?
        .trim();

    let mut parts = data.split(',');
    let registration: u8 = parts
        .next()
        .ok_or(anyhow!("Missing registration status"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid registration status"))?;

    let technology: u8 = parts
        .next()
        .ok_or(anyhow!("Missing technology status"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid technology status"))?;

    Ok((registration, technology))
}

pub fn parse_csq_response(response: &str) -> Result<(i32, i32)> {
    let csq_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CSQ:"))
        .ok_or(anyhow!("No CSQ response found in buffer"))?;

    let data = csq_line
        .trim()
        .strip_prefix("+CSQ:")
        .ok_or(anyhow!("Malformed CSQ response"))?
        .trim();

    let mut parts = data.split(',');
    let rssi: i32 = parts
        .next()
        .ok_or(anyhow!("Missing RSSI value"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid RSSI value"))?;

    let ber: i32 = parts
        .next()
        .ok_or(anyhow!("Missing BER value"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid BER value"))?;

    Ok((rssi, ber))
}

pub fn parse_cops_response(response: &str) -> Result<(u8, u8, String)> {
    let cops_line = response
        .lines()
        .find(|line| line.trim().starts_with("+COPS:"))
        .ok_or(anyhow!("No COPS response found in buffer"))?;

    let data = cops_line
        .trim()
        .strip_prefix("+COPS:")
        .ok_or(anyhow!("Malformed COPS response"))?
        .trim();

    let mut parts = data.split(',');
    let status: u8 = parts
        .next()
        .ok_or(anyhow!("Missing operator status"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid operator status"))?;

    let format: u8 = parts
        .next()
        .ok_or(anyhow!("Missing operator format"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid operator format"))?;

    let operator = parts
        .next()
        .ok_or(anyhow!("Missing operator name"))?
        .trim()
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(anyhow!("Operator name not properly quoted"))?
        .to_string();

    Ok((status, format, operator))
}

pub fn parse_cspn_response(response: &str) -> Result<String> {
    let cspn_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CSPN:"))
        .ok_or(anyhow!("No CSPN response found in buffer"))?;

    let data = cspn_line
        .trim()
        .strip_prefix("+CSPN:")
        .ok_or(anyhow!("Malformed CSPN response"))?
        .trim();

    // Find the quoted operator name.
    let quote_start = data.find('"').ok_or(anyhow!("Missing opening quote for operator name"))?;
    let quote_end = data.rfind('"').ok_or(anyhow!("Missing closing quote for operator name"))?;

    if quote_start >= quote_end {
        return Err(anyhow!("Invalid quoted operator name"));
    }
    Ok(data[quote_start + 1..quote_end].to_string())
}

pub fn parse_cbc_response(response: &str) -> Result<(u8, u8, f32)> {
    let cbc_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CBC:"))
        .ok_or(anyhow!("No CBC response found in buffer"))?;

    let data = cbc_line
        .trim()
        .strip_prefix("+CBC:")
        .ok_or(anyhow!("Malformed CBC response"))?
        .trim();

    let mut parts = data.split(',');
    let status: u8 = parts
        .next()
        .ok_or(anyhow!("Missing battery status"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid battery status"))?;

    let charge: u8 = parts
        .next()
        .ok_or(anyhow!("Missing battery charge"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid battery charge"))?;

    let voltage_raw: u32 = parts
        .next()
        .ok_or(anyhow!("Missing battery voltage"))?
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid battery voltage"))?;

    let voltage: f32 = voltage_raw as f32 / 1000.0;
    Ok((status, charge, voltage))
}

pub fn parse_cgpsstatus_response(response: &str) -> Result<GNSSFixStatus> {
    let cgps_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CGPSSTATUS:"))
        .ok_or(anyhow!("No CGPSSTATUS response found in buffer"))?;

    let status_str = cgps_line
        .split_once(": ")
        .map(|(_, s)| s.trim())
        .ok_or(anyhow!("Missing CGPS status"))?;

    GNSSFixStatus::try_from(status_str)
}

pub fn parse_cgnsinf_response(response: &str, unsolicited: bool) -> Result<GNSSLocation> {
    let header = if unsolicited { "+UGNSINF" } else { "+CGNSINF" };
    let cgnsinf_line = response
        .lines()
        .find(|line| line.trim().starts_with(header))
        .ok_or(anyhow!("No CGNSINF response found in buffer"))?;

    let data_str = cgnsinf_line
        .split_once(": ")
        .map(|(_, s)| s.trim())
        .ok_or(anyhow!("Missing CGNSINF data"))?;

    let fields: Vec<&str> = data_str.split(",").collect();
    GNSSLocation::try_from(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cmgs_result() {
        // Success cases
        let response = "AT+CMGS=10\r\n+CMGS: 123\r\nOK\r\n";
        assert_eq!(parse_cmgs_result(response).unwrap(), 123);

        let response = "AT+CMGS=10\r\n  +CMGS:   42  \r\nOK\r\n";
        assert_eq!(parse_cmgs_result(response).unwrap(), 42);

        let response = "Some other line\r\n+CMGS: 99\r\nAnother line\r\nOK\r\n";
        assert_eq!(parse_cmgs_result(response).unwrap(), 99);

        // Failure cases
        let response = "AT+CMGS=10\r\nOK\r\n";
        assert!(parse_cmgs_result(response).is_err());
        assert!(parse_cmgs_result(response).unwrap_err().to_string().contains("No CMGS response found"));

        let response = "+CMGS: abc\r\n";
        assert!(parse_cmgs_result(response).is_err());
        assert!(parse_cmgs_result(response).unwrap_err().to_string().contains("Invalid CMGS message reference number"));

        let response = "";
        assert!(parse_cmgs_result(response).is_err());
    }

    #[test]
    fn test_parse_creg_response() {
        // Success cases
        let response = "+CREG: 1,7\r\nOK\r\n";
        assert_eq!(parse_creg_response(response).unwrap(), (1, 7));

        let response = "  +CREG:  2 , 4  \r\nOK\r\n";
        assert_eq!(parse_creg_response(response).unwrap(), (2, 4));

        // Failure cases
        let response = "OK\r\n";
        assert!(parse_creg_response(response).is_err());
        assert!(parse_creg_response(response).unwrap_err().to_string().contains("No CREG response found"));

        let response = "+CREG: 1\r\n";
        assert!(parse_creg_response(response).is_err());
        assert!(parse_creg_response(response).unwrap_err().to_string().contains("Missing technology status"));

        let response = "+CREG: abc,7\r\n";
        assert!(parse_creg_response(response).is_err());
        assert!(parse_creg_response(response).unwrap_err().to_string().contains("Invalid registration status"));

        let response = "+CREG: 1,xyz\r\n";
        assert!(parse_creg_response(response).is_err());
        assert!(parse_creg_response(response).unwrap_err().to_string().contains("Invalid technology status"));
    }

    #[test]
    fn test_parse_csq_response() {
        // Success cases
        let response = "+CSQ: 15,99\r\nOK\r\n";
        assert_eq!(parse_csq_response(response).unwrap(), (15, 99));

        let response = "+CSQ: -50,-10\r\nOK\r\n";
        assert_eq!(parse_csq_response(response).unwrap(), (-50, -10));

        // Failure cases
        let response = "ERROR\r\n";
        assert!(parse_csq_response(response).is_err());
        assert!(parse_csq_response(response).unwrap_err().to_string().contains("No CSQ response found"));

        let response = "+CSQ: 15\r\n";
        assert!(parse_csq_response(response).is_err());
        assert!(parse_csq_response(response).unwrap_err().to_string().contains("Missing BER value"));

        let response = "+CSQ: abc,99\r\n";
        assert!(parse_csq_response(response).is_err());
        assert!(parse_csq_response(response).unwrap_err().to_string().contains("Invalid RSSI value"));

        let response = "+CSQ: 15,xyz\r\n";
        assert!(parse_csq_response(response).is_err());
        assert!(parse_csq_response(response).unwrap_err().to_string().contains("Invalid BER value"));

        let response = "\r\n\r\n\r\n";
        assert!(parse_csq_response(response).is_err());
    }

    #[test]
    fn test_parse_cops_response() {
        // Success cases
        let response = "+COPS: 0,2,\"Vodafone\"\r\nOK\r\n";
        let (status, format, operator) = parse_cops_response(response).unwrap();
        assert_eq!(status, 0);
        assert_eq!(format, 2);
        assert_eq!(operator, "Vodafone");

        let response = "+COPS: 1, 0, \"T-Mobile UK\"\r\nOK\r\n";
        let (status, format, operator) = parse_cops_response(response).unwrap();
        assert_eq!(status, 1);
        assert_eq!(format, 0);
        assert_eq!(operator, "T-Mobile UK");

        // Failure cases
        let response = "ERROR\r\n";
        assert!(parse_cops_response(response).is_err());
        assert!(parse_cops_response(response).unwrap_err().to_string().contains("No COPS response found"));

        let response = "+COPS: 0,2,Vodafone\r\n";
        assert!(parse_cops_response(response).is_err());
        assert!(parse_cops_response(response).unwrap_err().to_string().contains("Operator name not properly quoted"));

        let response = "+COPS: 0,2\r\n";
        assert!(parse_cops_response(response).is_err());
        assert!(parse_cops_response(response).unwrap_err().to_string().contains("Missing operator name"));

        let response = "+COPS: abc,2,\"Vodafone\"\r\n";
        assert!(parse_cops_response(response).is_err());
        assert!(parse_cops_response(response).unwrap_err().to_string().contains("Invalid operator status"));

        let response = "+COPS: 0,xyz,\"Vodafone\"\r\n";
        assert!(parse_cops_response(response).is_err());
        assert!(parse_cops_response(response).unwrap_err().to_string().contains("Invalid operator format"));
    }

    #[test]
    fn test_parse_cspn_response() {
        // Success cases
        let response = "+CSPN: \"EE\",0\r\nOK\r\n";
        assert_eq!(parse_cspn_response(response).unwrap(), "EE");

        let response = "+CSPN: \"Three UK\",1\r\nOK\r\n";
        assert_eq!(parse_cspn_response(response).unwrap(), "Three UK");

        // Failure cases
        let response = "ERROR\r\n";
        assert!(parse_cspn_response(response).is_err());
        assert!(parse_cspn_response(response).unwrap_err().to_string().contains("No CSPN response found"));

        let response = "+CSPN: EE,0\r\n";
        assert!(parse_cspn_response(response).is_err());
        assert!(parse_cspn_response(response).unwrap_err().to_string().contains("Missing opening quote"));

        let response = "+CSPN: \"EE,0\r\n";  // Missing closing quote
        assert!(parse_cspn_response(response).is_err());
        assert!(parse_cspn_response(response).unwrap_err().to_string().contains("Invalid quoted operator name"));

        let response = "+CSPN: \"\",0\r\n";  // Empty quotes (edge case)
        assert_eq!(parse_cspn_response(response).unwrap(), "");
    }

    #[test]
    fn test_parse_cbc_response() {
        // Success cases
        let response = "+CBC: 0,50,3800\r\nOK\r\n";
        let (status, charge, voltage) = parse_cbc_response(response).unwrap();
        assert_eq!(status, 0);
        assert_eq!(charge, 50);
        assert!((voltage - 3.8).abs() < f32::EPSILON);

        let response = "+CBC: 1,100,4123\r\nOK\r\n";
        let (status, charge, voltage) = parse_cbc_response(response).unwrap();
        assert_eq!(status, 1);
        assert_eq!(charge, 100);
        assert!((voltage - 4.123).abs() < f32::EPSILON);

        // Failure cases
        let response = "ERROR\r\n";
        assert!(parse_cbc_response(response).is_err());
        assert!(parse_cbc_response(response).unwrap_err().to_string().contains("No CBC response found"));

        let response = "+CBC: 0,50\r\n";
        assert!(parse_cbc_response(response).is_err());
        assert!(parse_cbc_response(response).unwrap_err().to_string().contains("Missing battery voltage"));

        let response = "+CBC: abc,50,3800\r\n";
        assert!(parse_cbc_response(response).is_err());
        assert!(parse_cbc_response(response).unwrap_err().to_string().contains("Invalid battery status"));

        let response = "+CBC: 0,xyz,3800\r\n";
        assert!(parse_cbc_response(response).is_err());
        assert!(parse_cbc_response(response).unwrap_err().to_string().contains("Invalid battery charge"));

        let response = "+CBC: 0,50,abc\r\n";
        assert!(parse_cbc_response(response).is_err());
        assert!(parse_cbc_response(response).unwrap_err().to_string().contains("Invalid battery voltage"));
    }

    #[test]
    fn test_parse_cgpsstatus_response() {
        // Success case
        let response = "+CGPSSTATUS: Location 3D Fix\r\nOK\r\n";
        assert!(parse_cgpsstatus_response(response).is_ok());

        // Failure cases
        let response = "ERROR\r\n";
        assert!(parse_cgpsstatus_response(response).is_err());
        assert!(parse_cgpsstatus_response(response).unwrap_err().to_string().contains("No CGPSSTATUS response found"));

        let response = "+CGPSSTATUS\r\n";  // Missing colon and status
        assert!(parse_cgpsstatus_response(response).is_err());
        assert!(parse_cgpsstatus_response(response).unwrap_err().to_string().contains("No CGPSSTATUS response found in buffer"));

        let response = "+CGPSSTATUS:\r\n";  // Empty status
        assert!(parse_cgpsstatus_response(response).is_err());
        assert!(parse_cgpsstatus_response(response).unwrap_err().to_string().contains("Missing CGPS status"));
    }

    #[test]
    fn test_parse_cgnsinf_response() {
        // Success
        let response = "+CGNSINF: 1,1,20230815120000.000,51.5074,-0.1278,85.4,0.0,0.0,1,0.9,1.2,0.8,,,10,4,,,42\r\nOK\r\n";
        assert!(parse_cgnsinf_response(response, false).is_ok());

        let response = "+UGNSINF: 1,1,20230815120000.000,51.5074,-0.1278,85.4,0.0,0.0,1,0.9,1.2,0.8,,,10,4,,,42\r\nOK\r\n";
        assert!(parse_cgnsinf_response(response, true).is_ok());
        /// TODO: Validate the location parsed values.

        // Failure cases
        let response = "+UGNSINF: data\r\nOK\r\n";  // Looking for +CGNSINF but only +UGNSINF present
        assert!(parse_cgnsinf_response(response, false).is_err());
        assert!(parse_cgnsinf_response(response, false).unwrap_err().to_string().contains("No CGNSINF response found"));

        let response = "+CGNSINF: data\r\nOK\r\n";  // Looking for +UGNSINF but only +CGNSINF present
        assert!(parse_cgnsinf_response(response, true).is_err());
        assert!(parse_cgnsinf_response(response, true).unwrap_err().to_string().contains("No CGNSINF response found"));

        let response = "+CGNSINF\r\n";  // Missing colon and data
        assert!(parse_cgnsinf_response(response, false).is_err());
        assert!(parse_cgnsinf_response(response, false).unwrap_err().to_string().contains("Missing CGNSINF data"));

        let response = "+UGNSINF\r\n";  // Missing colon and data (unsolicited)
        assert!(parse_cgnsinf_response(response, true).is_err());
        assert!(parse_cgnsinf_response(response, true).unwrap_err().to_string().contains("Missing CGNSINF data"));
    }
}
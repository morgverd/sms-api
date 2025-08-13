use anyhow::{anyhow, bail, Result};
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

pub fn parse_cgpsinf_response(response: &str) -> Result<GNSSLocation> {
    let cgps_line = response
        .lines()
        .find(|line| line.trim().starts_with("+CGPSINF:"))
        .ok_or(anyhow!("No CGPSINF response found in buffer"))?;

    let data_str = cgps_line
        .split_once(": ")
        .map(|(_, s)| s.trim())
        .ok_or(anyhow!("Missing CGPSINF data"))?;

    let fields: Vec<&str> = data_str.split(",").collect();
    if fields.len() < 14 {
        bail!("Insufficient GNSS data fields: got {}, expected 14", fields.len());
    }
    GNSSLocation::try_from(fields)
}
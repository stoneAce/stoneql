//! PostgreSQL options, identity check, read-only execution and lossless binary decoding.
use super::{
    DirectServer, LifecycleError, ReadError, connect_error,
    read_only::{QueryLimits, driver_error, finish, float_text, hex},
    ssh::Endpoint,
};
use sqlx::{
    ConnectOptions, PgPool, Row, TypeInfo, ValueRef,
    postgres::{
        PgConnectOptions, PgPoolOptions, PgRow, PgSslMode, PgTypeInfo, PgTypeKind, PgValueFormat,
    },
};
use std::time::Duration;
use stoneql_contracts::v1::{CellValue, ResultSet};
use stoneql_core::Engine;

/// libpq variables SQLx would silently apply to trust, client identity or session settings.
/// StoneQL refuses rather than inheriting unreviewed process configuration.
const INHERITED_ENVIRONMENT: [&str; 4] = ["PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY", "PGOPTIONS"];

/// `endpoint` is an owned SSH tunnel directory holding `.s.PGSQL.<port>`. TLS is still
/// requested over it and verified against the original host.
pub(crate) fn options(
    config: &DirectServer,
    endpoint: Option<&Endpoint>,
) -> Result<PgConnectOptions, LifecycleError> {
    config.validate()?;
    // A leading slash selects a Unix socket, which bypasses TLS verification entirely.
    if config.engine != Engine::PostgreSql
        || config.host.starts_with('/')
        || INHERITED_ENVIRONMENT
            .iter()
            .any(|name| std::env::var_os(name).is_some())
    {
        return Err(LifecycleError::InvalidConfiguration);
    }
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(&config.host)
        .port(config.port)
        .username(&config.username)
        .password(&config.password)
        .database(&config.database)
        .ssl_mode(PgSslMode::VerifyFull)
        .statement_cache_capacity(0)
        // A8 (approved 2026-11-16): backend messages are refused above 2 MiB from their header,
        // before the vendored driver buffers the payload.
        .max_receive_message_size(
            std::num::NonZeroUsize::new(2 * 1024 * 1024).expect("nonzero message limit"),
        )
        .application_name("StoneQL")
        .disable_statement_logging();
    if let Some(pem) = &config.root_certificate_pem {
        options = options.ssl_root_cert_from_pem(pem.clone());
    }
    if let Some(endpoint) = endpoint {
        options = options.socket(&endpoint.directory);
    }
    Ok(options)
}

pub(crate) async fn open(
    config: &DirectServer,
    endpoint: Option<&Endpoint>,
    deadline: Duration,
) -> Result<(PgPool, String), LifecycleError> {
    let options = options(config, endpoint)?;
    let pool = tokio::time::timeout(
        deadline,
        PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(deadline)
            .idle_timeout(crate::POOL_IDLE_TIMEOUT)
            .connect_with(options),
    )
    .await
    .map_err(|_| LifecycleError::Timeout)?
    .map_err(connect_error)?;
    let identity = tokio::time::timeout(
        deadline,
        sqlx::query_as::<_, (String, String)>(
            "SELECT version(), current_setting('server_version')",
        )
        .fetch_one(&pool),
    )
    .await;
    match identity {
        // Protocol-compatible servers (for example CockroachDB) are not PostgreSQL.
        Ok(Ok((banner, version))) if banner.starts_with("PostgreSQL ") => {
            Ok((pool, format!("PostgreSQL {version}")))
        }
        other => {
            pool.close().await;
            Err(match other {
                Err(_) => LifecycleError::Timeout,
                Ok(Err(_)) => LifecycleError::Unavailable,
                Ok(Ok(_)) => LifecycleError::EngineMismatch,
            })
        }
    }
}

pub(crate) async fn read(
    pool: &PgPool,
    sql: &str,
    limits: QueryLimits,
) -> Result<ResultSet, ReadError> {
    let connection = pool.acquire().await.map_err(driver_error)?;
    // The engine-enforced read-only transaction is the boundary. The server-side statement
    // timeout mirrors the client deadline so an abandoned statement does not keep running.
    finish(
        connection,
        &[
            "START TRANSACTION READ ONLY".into(),
            format!(
                "SET LOCAL statement_timeout = {}",
                limits.deadline.as_millis()
            ),
        ],
        sql,
        limits,
        decode_cell,
    )
    .await
}

pub(crate) fn decode_cell(row: &PgRow, index: usize) -> Result<CellValue, ReadError> {
    let raw = row.try_get_raw(index).map_err(|_| ReadError::Database)?;
    if raw.is_null() {
        return Ok(CellValue::Null);
    }
    let info = raw.type_info().into_owned();
    let name = info.name().to_owned();
    let bytes = raw.as_bytes().map_err(|_| ReadError::Database)?;
    let decoded = if raw.format() == PgValueFormat::Binary {
        decode_binary(&info, &name, bytes)
    } else {
        None
    };
    Ok(decoded.unwrap_or_else(|| CellValue::Unknown {
        database_type: name,
        hex: hex(bytes),
    }))
}

/// Exact binary decoders for reviewed built-ins. Anything else stays explicit raw hex.
fn decode_binary(info: &PgTypeInfo, name: &str, bytes: &[u8]) -> Option<CellValue> {
    let text = || String::from_utf8(bytes.to_vec()).ok();
    match info.kind() {
        // Enum labels are transmitted as their UTF-8 label bytes.
        PgTypeKind::Enum(_) => return text().map(|text| CellValue::Text { text }),
        PgTypeKind::Domain(base) => {
            return match decode_binary(base, base.name(), bytes)? {
                CellValue::Temporal { text, .. } => Some(CellValue::Temporal {
                    database_type: name.into(),
                    text,
                }),
                value => Some(value),
            };
        }
        _ => {}
    }
    let integer = |text: String| Some(CellValue::Integer { text });
    let temporal = |text: String| {
        Some(CellValue::Temporal {
            database_type: name.into(),
            text,
        })
    };
    match name {
        "BOOL" => match bytes {
            [value @ (0 | 1)] => Some(CellValue::Boolean { value: *value == 1 }),
            _ => None,
        },
        "INT2" => integer(i16::from_be_bytes(bytes.try_into().ok()?).to_string()),
        "INT4" => integer(i32::from_be_bytes(bytes.try_into().ok()?).to_string()),
        "INT8" => integer(i64::from_be_bytes(bytes.try_into().ok()?).to_string()),
        "OID" => integer(u32::from_be_bytes(bytes.try_into().ok()?).to_string()),
        "FLOAT4" => Some(CellValue::Float {
            text: float_text(f64::from(f32::from_be_bytes(bytes.try_into().ok()?))),
        }),
        "FLOAT8" => Some(CellValue::Float {
            text: float_text(f64::from_be_bytes(bytes.try_into().ok()?)),
        }),
        "NUMERIC" => numeric(bytes).map(|text| CellValue::Decimal { text }),
        // `"CHAR"` (quoted) is PostgreSQL's internal one-byte "char" type, used by catalog codes
        // such as pg_constraint.confdeltype; its byte is the character itself.
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "\"CHAR\"" | "UNKNOWN" => {
            text().map(|text| CellValue::Text { text })
        }
        "JSON" => text().map(|text| CellValue::Json { text }),
        "JSONB" => match bytes.split_first() {
            Some((1, json)) => String::from_utf8(json.to_vec())
                .ok()
                .map(|text| CellValue::Json { text }),
            _ => None,
        },
        "BYTEA" => Some(CellValue::Binary { hex: hex(bytes) }),
        "UUID" if bytes.len() == 16 => {
            let digits = hex(bytes);
            Some(CellValue::Text {
                text: format!(
                    "{}-{}-{}-{}-{}",
                    &digits[0..8],
                    &digits[8..12],
                    &digits[12..16],
                    &digits[16..20],
                    &digits[20..32]
                ),
            })
        }
        "DATE" => temporal(date(i32::from_be_bytes(bytes.try_into().ok()?))),
        "TIME" => temporal(clock(i64::from_be_bytes(bytes.try_into().ok()?))?),
        "TIMETZ" if bytes.len() == 12 => {
            let micros = i64::from_be_bytes(bytes[..8].try_into().ok()?);
            // PostgreSQL transmits the zone as seconds west of UTC.
            let west = i32::from_be_bytes(bytes[8..].try_into().ok()?);
            temporal(format!("{}{}", clock(micros)?, offset(-i64::from(west))))
        }
        "TIMESTAMP" => temporal(timestamp(i64::from_be_bytes(bytes.try_into().ok()?), "")?),
        // The stored value is an instant; render it in UTC with an explicit offset rather than
        // applying a local or session time zone.
        "TIMESTAMPTZ" => temporal(timestamp(
            i64::from_be_bytes(bytes.try_into().ok()?),
            "+00",
        )?),
        _ => None,
    }
}

/// PostgreSQL NUMERIC binary form: base-10000 digits, weight, sign and display scale.
fn numeric(bytes: &[u8]) -> Option<String> {
    let word = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let count = usize::from(word(0)?);
    let weight = i64::from(word(2)? as i16);
    let sign = word(4)?;
    let scale = usize::from(word(6)?);
    if bytes.len() != 8 + 2 * count {
        return None;
    }
    let digits = (0..count)
        .map(|index| word(8 + 2 * index).filter(|digit| *digit <= 9999))
        .collect::<Option<Vec<_>>>()?;
    let digit = |position: i64| {
        usize::try_from(position)
            .ok()
            .and_then(|position| digits.get(position).copied())
            .unwrap_or(0)
    };
    let mut text = match sign {
        0xC000 => return Some("NaN".into()),
        0xD000 => return Some("Infinity".into()),
        0xF000 => return Some("-Infinity".into()),
        0x4000 => String::from("-"),
        0x0000 => String::new(),
        _ => return None,
    };
    if weight < 0 {
        text.push('0');
    } else {
        text.push_str(&digit(0).to_string());
        for position in 1..=weight {
            text.push_str(&format!("{:04}", digit(position)));
        }
    }
    if scale > 0 {
        let mut fraction = String::with_capacity(scale + 4);
        let mut position = weight + 1;
        while fraction.len() < scale {
            fraction.push_str(&format!("{:04}", digit(position)));
            position += 1;
        }
        fraction.truncate(scale);
        text.push('.');
        text.push_str(&fraction);
    }
    Some(text)
}

const MICROS_PER_DAY: i64 = 86_400_000_000;
/// Days from 1970-01-01 to PostgreSQL's 2000-01-01 epoch.
const EPOCH_DAYS: i64 = 10_957;

/// Proleptic Gregorian civil date, using PostgreSQL's BC notation for years before 1.
fn civil(days_since_2000: i64) -> (String, bool) {
    let shifted = days_since_2000 + EPOCH_DAYS + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let (display, bc) = if year >= 1 {
        (year, false)
    } else {
        (1 - year, true)
    };
    (format!("{display:04}-{month:02}-{day:02}"), bc)
}
fn date(days: i32) -> String {
    match days {
        i32::MAX => "infinity".into(),
        i32::MIN => "-infinity".into(),
        days => {
            let (text, bc) = civil(i64::from(days));
            if bc { format!("{text} BC") } else { text }
        }
    }
}
/// Time of day with PostgreSQL's trailing-zero-trimmed fractional seconds.
fn clock(micros: i64) -> Option<String> {
    if !(0..=MICROS_PER_DAY).contains(&micros) {
        return None;
    }
    let seconds = micros / 1_000_000;
    let mut text = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    let fraction = micros % 1_000_000;
    if fraction != 0 {
        text.push_str(format!(".{fraction:06}").trim_end_matches('0'));
    }
    Some(text)
}
fn offset(east_seconds: i64) -> String {
    let sign = if east_seconds < 0 { '-' } else { '+' };
    let magnitude = east_seconds.abs();
    let (hours, minutes, seconds) = (magnitude / 3600, magnitude / 60 % 60, magnitude % 60);
    match (minutes, seconds) {
        (0, 0) => format!("{sign}{hours:02}"),
        (_, 0) => format!("{sign}{hours:02}:{minutes:02}"),
        _ => format!("{sign}{hours:02}:{minutes:02}:{seconds:02}"),
    }
}
fn timestamp(micros: i64, zone: &str) -> Option<String> {
    match micros {
        i64::MAX => Some("infinity".into()),
        i64::MIN => Some("-infinity".into()),
        micros => {
            let (day, bc) = civil(micros.div_euclid(MICROS_PER_DAY));
            let time = clock(micros.rem_euclid(MICROS_PER_DAY))?;
            Some(format!("{day} {time}{zone}{}", if bc { " BC" } else { "" }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn numeric_bytes(weight: i16, sign: u16, scale: u16, digits: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [digits.len() as u16, weight as u16, sign, scale] {
            bytes.extend(word.to_be_bytes());
        }
        for digit in digits {
            bytes.extend(digit.to_be_bytes());
        }
        bytes
    }
    #[test]
    fn numeric_text_preserves_scale_sign_and_special_values() {
        // 123456789012345678901234567890.4500 = groups 12|3456|7890|1234|5678|9012|3456|7890 . 4500
        let digits = [12, 3456, 7890, 1234, 5678, 9012, 3456, 7890, 4500];
        assert_eq!(
            numeric(&numeric_bytes(7, 0, 4, &digits)).as_deref(),
            Some("123456789012345678901234567890.4500")
        );
        assert_eq!(
            numeric(&numeric_bytes(-2, 0x4000, 6, &[1200])).as_deref(),
            Some("-0.000012")
        );
        assert_eq!(
            numeric(&numeric_bytes(0, 0, 2, &[])).as_deref(),
            Some("0.00")
        );
        assert_eq!(
            numeric(&numeric_bytes(1, 0, 0, &[1])).as_deref(),
            Some("10000")
        );
        assert_eq!(
            numeric(&numeric_bytes(0, 0xC000, 0, &[])).as_deref(),
            Some("NaN")
        );
        assert_eq!(
            numeric(&numeric_bytes(0, 0xF000, 0, &[])).as_deref(),
            Some("-Infinity")
        );
        assert!(numeric(&numeric_bytes(0, 0, 0, &[10_000])).is_none());
        assert!(numeric(&[0, 1, 0, 0]).is_none());
    }
    #[test]
    fn temporal_text_matches_postgresql_output_without_timezone_conversion() {
        assert_eq!(date(0), "2000-01-01");
        assert_eq!(date(8825), "2024-02-29");
        assert_eq!(date(-730_119), "0001-01-01");
        assert_eq!(date(-730_120), "0001-12-31 BC");
        assert_eq!(date(i32::MAX), "infinity");
        let micros = 8825 * MICROS_PER_DAY + 45_296_123_456;
        assert_eq!(
            timestamp(micros, "").as_deref(),
            Some("2024-02-29 12:34:56.123456")
        );
        assert_eq!(
            timestamp(micros - 23_456, "+00").as_deref(),
            Some("2024-02-29 12:34:56.1+00")
        );
        assert_eq!(
            timestamp(-730_120 * MICROS_PER_DAY, "").as_deref(),
            Some("0001-12-31 00:00:00 BC")
        );
        assert_eq!(timestamp(i64::MIN, "").as_deref(), Some("-infinity"));
        assert_eq!(clock(MICROS_PER_DAY).as_deref(), Some("24:00:00"));
        assert!(clock(-1).is_none());
        assert_eq!(offset(19_800), "+05:30");
        assert_eq!(offset(-3_600), "-01");
    }
}

//! The keys `Settings > Queue` writes (MOD-12 M2 D8-D10): two `project.settings` caps, three
//! `app_setting` rows and one `box.settings` key, each with its own validator, plus the USD and
//! window text the editor types. Nothing here touches a store.

use serde_json::Value;

use crate::model::quota::{CapError, PER_TOKEN_CAP_BATCH, PER_TOKEN_CAP_RUN};

str_enum!(
    /// MOD-12 M2 D9: one queue key, by its stored name. Which targets take which key is
    /// [`QueueSetting::PROJECT_KEYS`] / [`APP_KEYS`](QueueSetting::APP_KEYS) /
    /// [`BOX_KEYS`](QueueSetting::BOX_KEYS).
    QueueSetting {
        /// `project.settings.per_token_cap_run`, USD micros.
        PerTokenCapRun => "per_token_cap_run",
        /// `project.settings.per_token_cap_batch`, USD micros.
        PerTokenCapBatch => "per_token_cap_batch",
        /// `app_setting.min_budget_for_new_attempt`, USD micros.
        MinBudgetForNewAttempt => "min_budget_for_new_attempt",
        /// `app_setting.max_concurrent_items` and `box.settings.max_concurrent_items`.
        MaxConcurrentItems => "max_concurrent_items",
        /// `app_setting.scheduler_window` (D10): stored, not enforced.
        SchedulerWindow => "scheduler_window",
    }
);

/// D8: [`parse_usd`]'s refusal for text that is not a plain dollar amount.
pub const USD_NOT_A_NUMBER: &str = "type dollars, like 1.50";

/// D8: [`parse_usd`]'s refusal for more than six decimals.
pub const USD_TOO_PRECISE: &str = "at most six decimals: a micro-dollar is the smallest unit";

/// D8: [`parse_usd`]'s refusal for an amount whose micros do not fit an `i64`.
pub const USD_TOO_LARGE: &str = "that is more dollars than htui can count";

/// D10: [`parse_window`]'s refusal for text that is not `HH:MM-HH:MM`.
pub const WINDOW_SHAPE: &str = "type the window as HH:MM-HH:MM, like 22:00-06:00";

/// D10: [`parse_window`]'s refusal for a window that starts and ends on the same minute.
pub const WINDOW_EMPTY: &str = "the window starts and ends on the same minute";

/// Micros in one dollar.
const MICROS_PER_USD: i64 = 1_000_000;

impl QueueSetting {
    /// The keys a project target takes, in row order.
    pub const PROJECT_KEYS: [Self; 2] = [Self::PerTokenCapRun, Self::PerTokenCapBatch];
    /// The keys the app target takes, in row order.
    pub const APP_KEYS: [Self; 3] = [
        Self::MinBudgetForNewAttempt,
        Self::MaxConcurrentItems,
        Self::SchedulerWindow,
    ];
    /// The keys a box target takes.
    pub const BOX_KEYS: [Self; 1] = [Self::MaxConcurrentItems];

    /// Whether the editor types and shows this key in USD (D8): the caps and the minimum.
    #[must_use]
    pub const fn is_money(self) -> bool {
        matches!(
            self,
            Self::PerTokenCapRun | Self::PerTokenCapBatch | Self::MinBudgetForNewAttempt
        )
    }

    /// D9's validator for a value about to be **set** (a clear needs none).
    ///
    /// The caps take an integer `>= 0` (`0` is a real cap) and say
    /// [`CapError`]'s sentence, so a `null` is refused too: clearing is its own write. The minimum
    /// takes an integer `> 0`, the concurrency limit `1..=u32::MAX` (`BoxSettings` holds a
    /// `u32`), and the window `null` or D10's object.
    ///
    /// # Errors
    ///
    /// The sentence the section and the store both show, verbatim.
    pub fn validate(self, value: &Value) -> Result<(), String> {
        match self {
            Self::PerTokenCapRun | Self::PerTokenCapBatch => match value.as_i64() {
                Some(micros) if micros >= 0 => Ok(()),
                _ => Err(CapError::at_key(
                    if self == Self::PerTokenCapRun {
                        PER_TOKEN_CAP_RUN
                    } else {
                        PER_TOKEN_CAP_BATCH
                    },
                    value.to_string(),
                )
                .to_string()),
            },
            // Review R1: the section takes this in dollars, so an integer is read back in dollars
            // too, beside the micros it is stored as. Anything else is shown as written.
            Self::MinBudgetForNewAttempt => match value.as_i64() {
                Some(micros) if micros > 0 => Ok(()),
                found => Err(format!(
                    "app_setting.min_budget_for_new_attempt must be at least $0.000001, got {}",
                    found.map_or_else(
                        || value.to_string(),
                        |micros| format!("{} ({micros} USD micros)", format_usd(micros))
                    )
                )),
            },
            Self::MaxConcurrentItems => match value.as_u64() {
                Some(limit) if limit >= 1 && limit <= u64::from(u32::MAX) => Ok(()),
                _ => Err(format!(
                    "max_concurrent_items must be a whole number from 1 to {}, got {value}",
                    u32::MAX
                )),
            },
            Self::SchedulerWindow => {
                if value.is_null() || window_minutes(value).is_some() {
                    Ok(())
                } else {
                    Err(format!(
                        "scheduler_window must be null or {{\"start\":\"HH:MM\",\"end\":\"HH:MM\"}} \
                         with two different minutes, got {value}"
                    ))
                }
            }
        }
    }
}

/// D8: dollars as typed (`1.5`, `$1.50`, ` 0.000001 `) to USD micros.
///
/// Surrounding blanks and one leading `$` are ignored. A sign, an exponent, a thousands separator
/// and an empty string are refused: the section never parses empty text, because empty clears.
///
/// # Errors
///
/// [`USD_NOT_A_NUMBER`], [`USD_TOO_PRECISE`] (more than six decimals) or [`USD_TOO_LARGE`].
pub fn parse_usd(text: &str) -> Result<i64, String> {
    let text = text.trim();
    let text = text.strip_prefix('$').unwrap_or(text);
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return Err(USD_NOT_A_NUMBER.to_owned());
    }
    if fraction.len() > 6 {
        return Err(USD_TOO_PRECISE.to_owned());
    }
    let dollars = if whole.is_empty() {
        0
    } else {
        whole.parse::<i64>().map_err(|_| USD_TOO_LARGE.to_owned())?
    };
    let micros = if fraction.is_empty() {
        0
    } else {
        let padded = format!("{fraction:0<6}");
        padded
            .parse::<i64>()
            .expect("six ASCII digits always fit an i64")
    };
    dollars
        .checked_mul(MICROS_PER_USD)
        .and_then(|whole| whole.checked_add(micros))
        .ok_or_else(|| USD_TOO_LARGE.to_owned())
}

/// D8: USD micros as shown: `$` and at least two decimals, the rest trimmed of trailing zeros
/// (`1_500_000` → `$1.50`, `1_234_567` → `$1.234567`, `0` → `$0.00`). `parse_usd` inverts it for
/// every amount `>= 0`; a negative one (never stored by the validators) is shown with a leading `-`.
#[must_use]
pub fn format_usd(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let magnitude = micros.unsigned_abs();
    let per_usd = MICROS_PER_USD.unsigned_abs();
    let dollars = magnitude / per_usd;
    let fraction = format!("{:06}", magnitude % per_usd);
    let trimmed = fraction.trim_end_matches('0');
    let shown = if trimmed.len() < 2 {
        &fraction[..2]
    } else {
        trimmed
    };
    format!("{sign}${dollars}.{shown}")
}

/// D10: `HH:MM-HH:MM` to `{"start":"HH:MM","end":"HH:MM"}`; `end < start` crosses midnight.
///
/// Each side is exactly two hour digits (`00`-`23`), a colon and two minute digits (`00`-`59`);
/// blanks around the whole text or either side are ignored.
///
/// # Errors
///
/// [`WINDOW_SHAPE`], or [`WINDOW_EMPTY`] when start and end are the same minute.
pub fn parse_window(text: &str) -> Result<Value, String> {
    let (start, end) = text
        .trim()
        .split_once('-')
        .ok_or_else(|| WINDOW_SHAPE.to_owned())?;
    let (start, end) = (start.trim(), end.trim());
    let (Some(from), Some(to)) = (hh_mm(start), hh_mm(end)) else {
        return Err(WINDOW_SHAPE.to_owned());
    };
    if from == to {
        return Err(WINDOW_EMPTY.to_owned());
    }
    Ok(serde_json::json!({ "start": start, "end": end }))
}

/// D10: a stored window as typed back, `None` for `null`, absent, or a shape it does not know.
#[must_use]
pub fn format_window(value: &Value) -> Option<String> {
    window_minutes(value)?;
    let start = value.get("start")?.as_str()?;
    let end = value.get("end")?.as_str()?;
    Some(format!("{start}-{end}"))
}

/// D10's stored shape: an object with exactly `start` and `end`, each `HH:MM`, on two different
/// minutes. Answers both as minutes past midnight.
fn window_minutes(value: &Value) -> Option<(u16, u16)> {
    let map = value.as_object()?;
    if map.len() != 2 {
        return None;
    }
    let start = hh_mm(map.get("start")?.as_str()?)?;
    let end = hh_mm(map.get("end")?.as_str()?)?;
    (start != end).then_some((start, end))
}

/// `HH:MM` (exactly five characters, `00:00` to `23:59`) to minutes past midnight.
fn hh_mm(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    let pair = |hi: u8, lo: u8| -> Option<u16> {
        (hi.is_ascii_digit() && lo.is_ascii_digit())
            .then(|| u16::from(hi - b'0') * 10 + u16::from(lo - b'0'))
    };
    let hours = pair(bytes[0], bytes[1]).filter(|hours| *hours < 24)?;
    let minutes = pair(bytes[3], bytes[4]).filter(|minutes| *minutes < 60)?;
    Some(hours * 60 + minutes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_setting_texts_are_the_stored_keys() {
        let texts: Vec<&str> = QueueSetting::ALL.iter().map(|key| key.as_str()).collect();
        assert_eq!(
            texts,
            [
                "per_token_cap_run",
                "per_token_cap_batch",
                "min_budget_for_new_attempt",
                "max_concurrent_items",
                "scheduler_window",
            ]
        );
        assert_eq!(
            QueueSetting::PROJECT_KEYS,
            [QueueSetting::PerTokenCapRun, QueueSetting::PerTokenCapBatch]
        );
        assert_eq!(
            QueueSetting::APP_KEYS,
            [
                QueueSetting::MinBudgetForNewAttempt,
                QueueSetting::MaxConcurrentItems,
                QueueSetting::SchedulerWindow,
            ]
        );
        assert_eq!(QueueSetting::BOX_KEYS, [QueueSetting::MaxConcurrentItems]);
        assert_eq!(QueueSetting::PerTokenCapRun.as_str(), PER_TOKEN_CAP_RUN);
        assert_eq!(QueueSetting::PerTokenCapBatch.as_str(), PER_TOKEN_CAP_BATCH);
        let money: Vec<QueueSetting> = QueueSetting::ALL
            .iter()
            .copied()
            .filter(|key| key.is_money())
            .collect();
        assert_eq!(
            money,
            [
                QueueSetting::PerTokenCapRun,
                QueueSetting::PerTokenCapBatch,
                QueueSetting::MinBudgetForNewAttempt,
            ]
        );
    }

    #[test]
    fn caps_take_a_non_negative_integer_and_say_cap_errors_sentence() {
        for key in QueueSetting::PROJECT_KEYS {
            assert_eq!(key.validate(&json!(0)), Ok(()), "{key}: 0 is a real cap");
            assert_eq!(key.validate(&json!(1_500_000)), Ok(()), "{key}");
        }
        for (value, found) in [
            (json!(-1), "-1"),
            (json!(1.5), "1.5"),
            (json!("1"), "\"1\""),
            (Value::Null, "null"),
        ] {
            assert_eq!(
                QueueSetting::PerTokenCapBatch.validate(&value),
                Err(format!(
                    "project.settings.per_token_cap_batch must be a non-negative integer of USD \
                     micros, got {found}"
                ))
            );
        }
        assert_eq!(
            QueueSetting::PerTokenCapRun.validate(&json!(-1)),
            Err(
                "project.settings.per_token_cap_run must be a non-negative integer of USD micros, \
                 got -1"
                    .to_owned()
            )
        );
    }

    #[test]
    fn min_budget_takes_a_positive_integer() {
        let key = QueueSetting::MinBudgetForNewAttempt;
        assert_eq!(key.validate(&json!(1)), Ok(()));
        // Review R1: the section takes dollars, so an integer is also read back in dollars; the
        // stored unit stays named.
        for (value, found) in [
            (json!(0), "$0.00 (0 USD micros)"),
            (json!(-1), "-$0.000001 (-1 USD micros)"),
            (json!("5"), "\"5\""),
            (json!(1.5), "1.5"),
        ] {
            assert_eq!(
                key.validate(&value),
                Err(format!(
                    "app_setting.min_budget_for_new_attempt must be at least $0.000001, got \
                     {found}"
                )),
                "{value}"
            );
        }
    }

    #[test]
    fn max_concurrent_items_takes_at_least_one() {
        let key = QueueSetting::MaxConcurrentItems;
        assert_eq!(key.validate(&json!(1)), Ok(()));
        assert_eq!(key.validate(&json!(u32::MAX)), Ok(()));
        for (value, found) in [
            (json!(0), "0".to_owned()),
            (
                json!(u64::from(u32::MAX) + 1),
                (u64::from(u32::MAX) + 1).to_string(),
            ),
            (json!(2.0), "2.0".to_owned()),
        ] {
            assert_eq!(
                key.validate(&value),
                Err(format!(
                    "max_concurrent_items must be a whole number from 1 to 4294967295, got {found}"
                ))
            );
        }
    }

    #[test]
    fn the_window_takes_hh_mm_pairs_and_may_cross_midnight() {
        let window = parse_window("22:00-06:00").expect("a window across midnight parses");
        assert_eq!(window, json!({"start": "22:00", "end": "06:00"}));
        assert_eq!(QueueSetting::SchedulerWindow.validate(&window), Ok(()));
        assert_eq!(format_window(&window).as_deref(), Some("22:00-06:00"));
        assert_eq!(
            parse_window(" 09:30 - 17:45 "),
            Ok(json!({"start": "09:30", "end": "17:45"}))
        );
        for text in [
            "24:00-01:00",
            "9:00-10:00",
            "10:00",
            "10:60-11:00",
            "",
            "ab:cd-01:00",
        ] {
            assert_eq!(parse_window(text), Err(WINDOW_SHAPE.to_owned()), "{text:?}");
        }
        assert_eq!(parse_window("10:00-10:00"), Err(WINDOW_EMPTY.to_owned()));

        assert_eq!(QueueSetting::SchedulerWindow.validate(&Value::Null), Ok(()));
        for bad in [
            json!({"start": "10:00", "end": "10:00"}),
            json!({"start": "10:00"}),
            json!({"start": "10:00", "end": "11:00", "tz": "UTC"}),
            json!("22:00-06:00"),
            json!(1),
        ] {
            assert_eq!(
                QueueSetting::SchedulerWindow.validate(&bad),
                Err(format!(
                    "scheduler_window must be null or {{\"start\":\"HH:MM\",\"end\":\"HH:MM\"}} \
                     with two different minutes, got {bad}"
                ))
            );
            assert_eq!(format_window(&bad), None, "{bad}");
        }
        assert_eq!(format_window(&Value::Null), None);
    }

    #[test]
    fn usd_parses_to_micros_and_formats_back() {
        assert_eq!(parse_usd("1.5"), Ok(1_500_000));
        assert_eq!(parse_usd("$1.50"), Ok(1_500_000));
        assert_eq!(parse_usd(" 0.000001 "), Ok(1));
        assert_eq!(parse_usd("12"), Ok(12_000_000));
        assert_eq!(parse_usd(".25"), Ok(250_000));
        assert_eq!(parse_usd("1.2345678"), Err(USD_TOO_PRECISE.to_owned()));
        for text in [
            "-1", "1e3", "abc", "", "$", ".", "+1", "1,000", "1.2.3", "$-1",
        ] {
            assert_eq!(
                parse_usd(text),
                Err(USD_NOT_A_NUMBER.to_owned()),
                "{text:?}"
            );
        }
        assert_eq!(
            parse_usd("9223372036855"),
            Err(USD_TOO_LARGE.to_owned()),
            "past i64::MAX micros"
        );
        assert_eq!(
            parse_usd("99999999999999999999"),
            Err(USD_TOO_LARGE.to_owned())
        );

        assert_eq!(format_usd(1_500_000), "$1.50");
        assert_eq!(format_usd(1_234_567), "$1.234567");
        assert_eq!(format_usd(0), "$0.00");
        assert_eq!(format_usd(1), "$0.000001");
        assert_eq!(format_usd(10_000_000), "$10.00");
        for micros in [
            0,
            1,
            10,
            100_000,
            999_999,
            1_000_000,
            1_500_000,
            1_234_567,
            42_010_000,
            i64::MAX,
        ] {
            assert_eq!(parse_usd(&format_usd(micros)), Ok(micros), "{micros}");
        }
    }
}

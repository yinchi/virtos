/// Global instance of the Real-Time Clock (RTC) accessor using the ARM PL031 driver.
pub static RTC: spin::Once<arm_pl031::Rtc> = spin::Once::new();

/// Returns a reference to the global RTC accessor
pub fn rtc<'a>() -> &'a arm_pl031::Rtc {
    RTC.wait()
}

/// Timezone used during initialization, until the preferred timezone can be determined from the
/// environment ($TZ variable).
pub const DEFAULT_TIMEZONE: chrono_tz::Tz = chrono_tz::America::Toronto;

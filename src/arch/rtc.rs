pub static RTC: spin::Once<arm_pl031::Rtc> = spin::Once::new();

pub fn rtc<'a>() -> &'a arm_pl031::Rtc {
    RTC.wait()
}

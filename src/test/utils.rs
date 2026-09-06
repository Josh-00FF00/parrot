use std::time::Duration;

use crate::utils::{
    compare_domains, get_human_readable_timestamp, parse_netscape_cookie_line, parse_timestamp,
};

#[test]
fn test_get_human_readable_timestamp() {
    let duration = Duration::from_secs(53);
    let result = get_human_readable_timestamp(Some(duration));
    assert_eq!(result, "00:53");

    let duration = Duration::from_secs(3599);
    let result = get_human_readable_timestamp(Some(duration));
    assert_eq!(result, "59:59");

    let duration = Duration::from_secs(96548);
    let result = get_human_readable_timestamp(Some(duration));
    assert_eq!(result, "26:49:08");

    let result = get_human_readable_timestamp(Some(Duration::MAX));
    assert_eq!(result, "∞");

    let result = get_human_readable_timestamp(None);
    assert_eq!(result, "∞");
}

#[test]
fn test_parse_timestamp() {
    assert_eq!(parse_timestamp("53"), Some(53));
    assert_eq!(parse_timestamp("01:02"), Some(62));
    assert_eq!(parse_timestamp("1:02:03"), Some(3723));
    assert_eq!(parse_timestamp("01:02:03:04"), None);
    assert_eq!(parse_timestamp(""), None);
    assert_eq!(parse_timestamp("a:b"), None);
    assert_eq!(parse_timestamp(":-1"), None);
}

#[test]
fn test_compare_domains() {
    assert!(compare_domains("youtube.com", "youtube.com"));
    assert!(compare_domains("youtube.com", "www.youtube.com"));
    assert!(compare_domains("youtube.com", "music.youtube.com"));
    assert!(compare_domains(".youtube.com", "www.youtube.com"));
    assert!(compare_domains("YOUTUBE.com", "WWW.YOUTUBE.COM"));

    assert!(!compare_domains("youtube.com", "evilyoutube.com"));
    assert!(!compare_domains("youtube.com", "evil-youtube.com"));
    assert!(!compare_domains("youtube.com", "youtube.com.evil.com"));
    assert!(!compare_domains(
        "youtube.com",
        "youtube.com.evil.com.youtube.org"
    ));
}

#[test]
fn test_parse_netscape_cookie_line() {
    let line = ".youtube.com\tTRUE\t/\tTRUE\t0\tYSC\tvalue123";
    let cookie = parse_netscape_cookie_line(line).unwrap();

    assert_eq!(cookie.domain, ".youtube.com");
    assert_eq!(cookie.path, "/");
    assert!(cookie.secure);
    assert_eq!(cookie.name, "YSC");
    assert_eq!(cookie.value, "value123");

    let line = "youtube.com\tFALSE\t/\tFALSE\t0\tPREF\tvalue456";
    let cookie = parse_netscape_cookie_line(line).unwrap();

    assert_eq!(cookie.domain, "youtube.com");
    assert!(!cookie.secure);
    assert_eq!(cookie.name, "PREF");

    assert!(parse_netscape_cookie_line("invalid line").is_none());
}

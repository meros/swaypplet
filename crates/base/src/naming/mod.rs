//! Names a person reads for hardware: the vendor part shared by every
//! device, and the names of displays (`display`). Pure string rules, so the
//! shell, the services and the settings all use the same ones.

pub mod display;

/// A company's name without its legal form: "Realtek Semiconductor Corp."
/// is "Realtek", "Intel Corporation" is "Intel", "Samsung Display Corp." is
/// "Samsung Display". Cut at the first such word, never down to nothing.
pub fn short_vendor(vendor: &str) -> String {
    const NOISE: [&str; 14] = [
        "Semiconductor",
        "Corporation",
        "Corp.",
        "Corp",
        "Co.,",
        "Co.",
        "Ltd.",
        "Ltd",
        "Inc.",
        "Inc",
        "Technology",
        "Technologies",
        "Limited",
        "GmbH",
    ];
    let words: Vec<&str> = vendor
        .split_whitespace()
        .map(|w| w.trim_end_matches(','))
        .collect();
    let keep = words
        .iter()
        .position(|w| NOISE.iter().any(|n| n.trim_end_matches(',') == *w))
        .filter(|&i| i > 0)
        .unwrap_or(words.len());
    words[..keep].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_forms_are_dropped_from_vendors() {
        assert_eq!(short_vendor("Realtek Semiconductor Corp."), "Realtek");
        assert_eq!(short_vendor("Intel Corporation"), "Intel");
        assert_eq!(short_vendor("Samsung Display Corp."), "Samsung Display");
        assert_eq!(short_vendor("ASIX Electronics Corp."), "ASIX Electronics");
        assert_eq!(
            short_vendor("Hon Hai Precision Ind. Co., Ltd."),
            "Hon Hai Precision Ind."
        );
        assert_eq!(short_vendor("Dell Inc."), "Dell");
        assert_eq!(short_vendor("Lenovo"), "Lenovo");
    }
}

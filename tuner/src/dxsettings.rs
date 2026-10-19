//! Windows keeps several graphics switches in one registry string:
//! HKCU\Software\Microsoft\DirectX\UserGpuPreferences\DirectXUserGlobalSettings
//! = "VRROptimizeEnable=1;SwapEffectUpgradeEnable=1;"
//! These helpers read and change one key without disturbing the others.
//! Pure logic.

pub const KEY_PATH: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
pub const VALUE_NAME: &str = "DirectXUserGlobalSettings";
/// "Optimizations for windowed games" (flip-model upgrade): lower latency,
/// and lets VRR/G-SYNC work in borderless windowed games.
pub const SWAP_EFFECT_UPGRADE: &str = "SwapEffectUpgradeEnable";
/// "Variable refresh rate" for games that do not support VRR themselves.
pub const VRR_OPTIMIZE: &str = "VRROptimizeEnable";

pub fn parse(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

pub fn get(s: &str, key: &str) -> Option<String> {
    parse(s)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

/// Set `key=value`, keeping every other entry and its order.
pub fn set(s: &str, key: &str, value: &str) -> String {
    let mut items = parse(s);
    match items.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
        Some(item) => item.1 = value.to_string(),
        None => items.push((key.to_string(), value.to_string())),
    }
    let mut out = String::new();
    for (k, v) in items {
        out.push_str(&k);
        out.push('=');
        out.push_str(&v);
        out.push(';');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_keeps_other_keys() {
        let s = "VRROptimizeEnable=0;AutoHDREnable=1;";
        let t = set(s, SWAP_EFFECT_UPGRADE, "1");
        assert_eq!(t, "VRROptimizeEnable=0;AutoHDREnable=1;SwapEffectUpgradeEnable=1;");
        let u = set(&t, "vrroptimizeenable", "1");
        assert_eq!(u, "VRROptimizeEnable=1;AutoHDREnable=1;SwapEffectUpgradeEnable=1;");
        assert_eq!(get(&u, VRR_OPTIMIZE).as_deref(), Some("1"));
        assert_eq!(get("", VRR_OPTIMIZE), None);
        assert_eq!(set("", VRR_OPTIMIZE, "1"), "VRROptimizeEnable=1;");
    }

    #[test]
    fn tolerates_messy_strings() {
        let s = " A=1 ;;B= 2;C";
        assert_eq!(parse(s), vec![("A".into(), "1".into()), ("B".into(), "2".into()), ("C".into(), "".into())]);
    }
}

use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

/// Resolve a human-readable region for a host address (IP or hostname).
/// Private / loopback addresses return a local label; public IPs use ip-api.com.
pub async fn resolve_region(address: &str) -> String {
    let host = extract_host(address);
    if host.is_empty() {
        return String::new();
    }

    let ip = match resolve_ip(&host).await {
        Some(ip) => ip,
        None => return String::new(),
    };

    if is_private_or_local(ip) {
        return "内网".to_string();
    }

    lookup_public_ip(ip).await.unwrap_or_default()
}

/// Resolve the monitor machine's location. A container or private network
/// address cannot be geolocated directly, so use the requester's public
/// egress address in that case.
pub async fn resolve_local_region(address: &str) -> String {
    let host = extract_host(address);
    let ip = resolve_ip(&host).await;

    match ip {
        Some(ip) if !is_private_or_local(ip) => lookup_public_ip(ip).await.unwrap_or_default(),
        _ => lookup_requester_location().await.unwrap_or_default(),
    }
}

fn extract_host(address: &str) -> String {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Strip scheme if pasted as URL
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    // host:port or bare host / IPv6 in brackets
    if let Some(rest) = without_scheme.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return rest[..end].to_string();
    }
    // IPv4 or hostname with optional :port
    if without_scheme.matches(':').count() == 1
        && let Some((h, port)) = without_scheme.rsplit_once(':')
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return h.to_string();
    }
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

async fn resolve_ip(host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let host_owned = host.to_string();
    tokio::task::spawn_blocking(move || {
        (host_owned.as_str(), 0u16)
            .to_socket_addrs()
            .ok()?
            .next()
            .map(|a| a.ip())
    })
    .await
    .ok()
    .flatten()
}

fn is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct IpApiResponse {
    status: String,
    #[serde(default)]
    country: String,
    #[serde(default, rename = "regionName")]
    region_name: String,
    #[serde(default)]
    city: String,
}

async fn lookup_public_ip(ip: IpAddr) -> Option<String> {
    let url =
        format!("http://ip-api.com/json/{ip}?fields=status,country,regionName,city&lang=zh-CN");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: IpApiResponse = resp.json().await.ok()?;
    if body.status != "success" {
        return None;
    }
    Some(format_region(&body.country, &body.region_name, &body.city))
}

async fn lookup_requester_location() -> Option<String> {
    let url = "http://ip-api.com/json/?fields=status,country,regionName,city&lang=zh-CN";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: IpApiResponse = resp.json().await.ok()?;
    (body.status == "success").then(|| format_region(&body.country, &body.region_name, &body.city))
}

fn format_region(country: &str, region: &str, city: &str) -> String {
    let country = country.trim();
    let region = region.trim();
    let city = city.trim();

    // Prefer concise Chinese-friendly labels
    let mut parts: Vec<&str> = Vec::new();
    if !country.is_empty() {
        parts.push(country);
    }
    if !region.is_empty() && region != country {
        parts.push(region);
    }
    if !city.is_empty() && city != region && city != country {
        parts.push(city);
    }
    if parts.is_empty() {
        return String::new();
    }
    // e.g. 日本 · 东京都 · 东京  or  中国 · 香港
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_ipv4_port() {
        assert_eq!(extract_host("1.2.3.4:22"), "1.2.3.4");
    }

    #[test]
    fn extract_host_plain() {
        assert_eq!(extract_host(" example.com "), "example.com");
    }
}

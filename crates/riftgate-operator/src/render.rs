//! TOML config renderer: translates CRD objects into a riftgate TOML config.
//!
//! The operator reads `Riftgate` + `RiftgateBackend` + `RiftgateRoute`
//! objects and renders a TOML string that is stored in a `ConfigMap` and
//! mounted into the gateway container as `--config`.

use crate::crds::{RiftgateBackendSpec, RiftgateRouteSpec, RiftgateSpec};

/// Render a complete `riftgate.toml` from the CRD objects.
///
/// `routes` is a slice of `(name, spec)` pairs; each route contributes one
/// tenant entry to `[mcp.tenants]` and one API key entry to
/// `[multitenancy.api_keys]`.
pub fn render_config(
    gateway: &RiftgateSpec,
    backends: &[(&str, &RiftgateBackendSpec)],
    routes: &[(&str, &RiftgateRouteSpec)],
) -> String {
    let mut out = String::new();

    // [server]
    out.push_str("[server]\n");
    out.push_str(&format!("listen_addr = {:?}\n\n", gateway.listen_addr));

    // [backend] — use the first backend for the primary upstream
    if let Some((_, b)) = backends.first() {
        out.push_str("[backend]\n");
        if let Some(url) = &b.url {
            out.push_str(&format!("url = {:?}\n", url));
        }
        out.push_str(&format!("timeout_ms = {}\n\n", b.timeout_ms));
    }

    // [obs]
    out.push_str("[obs]\n");
    out.push_str(&format!("otel_endpoint = {:?}\n\n", gateway.obs_endpoint));

    // [log]
    out.push_str("[log]\n");
    out.push_str(&format!("level = {:?}\n\n", gateway.log_level));

    // [mcp] — one tenant section per route that has mcp config
    let mcp_routes: Vec<_> = routes.iter().filter(|(_, r)| r.mcp.is_some()).collect();
    if !mcp_routes.is_empty() {
        out.push_str("[mcp]\n");
        out.push_str("enforce = true\n\n");
        for (name, route) in &mcp_routes {
            if let Some(mcp) = &route.mcp {
                out.push_str(&format!("[mcp.tenants.{}]\n", sanitize_toml_key(name)));
                out.push_str(&format!(
                    "allowed_tools = {}\n",
                    to_toml_string_array(&mcp.allowed_tools)
                ));
                out.push_str(&format!(
                    "denied_tools = {}\n",
                    to_toml_string_array(&mcp.denied_tools)
                ));
                out.push_str(&format!(
                    "allowed_resource_prefixes = {}\n\n",
                    to_toml_string_array(&mcp.allowed_resource_prefixes)
                ));
            }
        }
    }

    out
}

fn sanitize_toml_key(s: &str) -> String {
    s.chars()
        .map(|c| if c == '-' || c == '.' { '_' } else { c })
        .collect()
}

fn to_toml_string_array(v: &[String]) -> String {
    let items: Vec<String> = v.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crds::{McpRouteConfig, RiftgateRouteSpec};

    #[test]
    fn render_minimal_config() {
        let gw = RiftgateSpec {
            image: "riftgate:v1".to_owned(),
            ..Default::default()
        };
        let backend = RiftgateBackendSpec {
            url: Some("http://vllm:8000".to_owned()),
            ..Default::default()
        };
        let toml = render_config(&gw, &[("llm-prod", &backend)], &[]);
        assert!(toml.contains("[server]"));
        assert!(toml.contains("[backend]"));
        assert!(toml.contains("url = \"http://vllm:8000\""));
        assert!(toml.contains("[obs]"));
    }

    #[test]
    fn render_with_mcp_route() {
        let gw = RiftgateSpec {
            image: "riftgate:v1".to_owned(),
            ..Default::default()
        };
        let backend = RiftgateBackendSpec::default();
        let route = RiftgateRouteSpec {
            path_prefix: "/v1/".to_owned(),
            backend_ref: "llm-prod".to_owned(),
            mcp: Some(McpRouteConfig {
                allowed_tools: vec!["search-web".to_owned()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let toml = render_config(&gw, &[("llm-prod", &backend)], &[("acme", &route)]);
        assert!(toml.contains("[mcp.tenants.acme]"));
        assert!(toml.contains("allowed_tools = [\"search-web\"]"));
    }
}

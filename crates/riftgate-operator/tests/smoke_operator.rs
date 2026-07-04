//! Operator smoke test (ADR 0030 Compliance).
//!
//! This test is `#[ignore]` — it requires a live Kubernetes cluster with
//! CRDs installed. Run it with:
//!
//!   # Install CRDs first:
//!   kubectl apply -f deploy/helm/riftgate-operator/templates/crds.yaml
//!
//!   # Then run with a kubeconfig pointing at the cluster:
//!   cargo test -p riftgate-operator --test smoke_operator -- --include-ignored
//!
//! In CI, this test is exercised via the `envtest` job (not yet wired; see
//! ADR 0030 Compliance for the planned test structure).

#[cfg(feature = "operator")]
mod operator_smoke {
    use riftgate_operator::crds::RiftgateSpec;

    /// Apply a `Riftgate` CR and assert the operator creates the expected
    /// `ConfigMap` and `Deployment`.
    ///
    /// `#[ignore]` — requires a live cluster with CRDs installed.
    #[tokio::test]
    #[ignore = "requires a live Kubernetes cluster with riftgate.io CRDs installed"]
    async fn riftgate_cr_creates_configmap_and_deployment() {
        use kube::api::{ObjectMeta, PostParams};
        use kube::{Api, Client};
        use riftgate_operator::crds::Riftgate;

        let client = Client::try_default()
            .await
            .expect("cannot connect to Kubernetes API");
        let ns = "default";

        // Create a Riftgate CR.
        let gateways: Api<Riftgate> = Api::namespaced(client.clone(), ns);
        let cr = Riftgate::new(
            "smoke-gateway",
            RiftgateSpec {
                image: "ghcr.io/sgpopuri/riftgate:latest".to_owned(),
                ..Default::default()
            },
        );
        gateways
            .create(&PostParams::default(), &cr)
            .await
            .expect("create Riftgate CR");

        // Wait for the ConfigMap to appear (up to 30s).
        let cms: Api<k8s_openapi::api::core::v1::ConfigMap> = Api::namespaced(client.clone(), ns);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if cms.get("smoke-gateway-config").await.is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for ConfigMap smoke-gateway-config"
            );
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        // Assert the Deployment also exists.
        let deploys: Api<k8s_openapi::api::apps::v1::Deployment> =
            Api::namespaced(client.clone(), ns);
        assert!(
            deploys.get("smoke-gateway").await.is_ok(),
            "Deployment smoke-gateway not created"
        );

        // Clean up.
        gateways
            .delete("smoke-gateway", &kube::api::DeleteParams::default())
            .await
            .ok();
    }
}

#[cfg(not(feature = "operator"))]
#[test]
fn smoke_test_requires_operator_feature() {
    // This test always passes as a placeholder. The real smoke test above
    // requires `--features operator` and a live cluster.
}

//! API key checks via free probe requests: the Anthropic model list, the OpenRouter key info.
use crate::error::{checked, net, EngineError};

const ANTHROPIC_MODELS: &str = "https://api.anthropic.com/v1/models";
/// Key info (limit, spend) — free, no model request.
const OPENROUTER_KEY: &str = "https://openrouter.ai/api/v1/key";

/// Probe request with the key `key`: a success means the key works, a refusal is an error by status,
/// a client timeout is `Timeout`. The key never appears in the error text, even if the service echoed it back.
async fn probe(req: reqwest::RequestBuilder, key: &str) -> Result<(), EngineError> {
    let res = match req.send().await {
        Ok(resp) => checked(resp, key).await.map(drop),
        Err(e) if e.is_timeout() => Err(EngineError::Timeout),
        Err(e) => Err(net(e)),
    };
    res.map_err(|e| e.redact(key))
}

/// Anthropic key: the model list.
pub async fn anthropic(http: &reqwest::Client, key: &str) -> Result<(), EngineError> {
    probe(
        http.get(ANTHROPIC_MODELS)
            .header("x-api-key", key)
            .header("anthropic-version", crate::claude::API_VERSION),
        key,
    )
    .await
}

/// OpenRouter key: info about the key itself.
pub async fn openrouter(http: &reqwest::Client, key: &str) -> Result<(), EngineError> {
    probe(openrouter_request(http, OPENROUTER_KEY, key), key).await
}

fn openrouter_request(http: &reqwest::Client, url: &str, key: &str) -> reqwest::RequestBuilder {
    http.get(url).bearer_auth(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fake_api, reply, Reply};
    use std::time::Duration;

    const KEY: &str = "sk-CHECK-0123456789";

    async fn check_at(url: &str, timeout: Duration) -> Result<(), EngineError> {
        let http = reqwest::Client::builder().no_proxy().timeout(timeout).build().unwrap();
        probe(http.get(url).header("x-api-key", KEY), KEY).await
    }

    const LONG: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn success_means_working_key() {
        let api = fake_api(|_, _, _| reply(200, r#"{"data":[]}"#)).await;
        assert_eq!(check_at(&api.base, LONG).await, Ok(()));
    }

    #[tokio::test]
    async fn refusal_is_key_problem_without_the_key() {
        let api = fake_api(|_, _, _| reply(401, &format!(r#"{{"error":{{"message":"invalid x-api-key {KEY}"}}}}"#))).await;
        let e = check_at(&api.base, LONG).await.unwrap_err();
        assert!(e.is_key_problem(), "{e:?}");
        let shown = format!("{e} {e:?}");
        assert!(shown.contains("401") && !shown.contains("CHECK"), "{shown}");
    }

    #[tokio::test]
    async fn openrouter_probe_reads_key_info_with_bearer() {
        let api = fake_api(|_, _, _| reply(200, r#"{"data":{"label":"sk-or-v1-abc...xyz","limit":null}}"#)).await;
        let http = reqwest::Client::builder().no_proxy().timeout(LONG).build().unwrap();
        let url = format!("{}/api/v1/key", api.base);
        assert_eq!(probe(openrouter_request(&http, &url, KEY), KEY).await, Ok(()));
        assert_eq!(api.calls(), ["GET /api/v1/key"], "a free read of the key info, not a model request");
        assert_eq!(api.seen()[0].header("authorization"), [format!("Bearer {KEY}").as_str()]);
        assert_eq!(OPENROUTER_KEY, "https://openrouter.ai/api/v1/key");
    }

    #[tokio::test]
    async fn openrouter_refusal_is_key_problem_without_the_key() {
        let body = format!(r#"{{"error":{{"code":401,"message":"No auth credentials found {KEY}"}}}}"#);
        let api = fake_api(move |_, _, _| reply(401, &body)).await;
        let http = reqwest::Client::builder().no_proxy().timeout(LONG).build().unwrap();
        let e = probe(openrouter_request(&http, &api.base, KEY), KEY).await.unwrap_err();
        assert!(e.is_key_problem(), "{e:?}");
        let shown = format!("{e} {e:?}");
        assert!(shown.contains("401") && !shown.contains("CHECK"), "{shown}");
    }

    #[tokio::test]
    async fn silent_service_is_timeout() {
        // The network "ate" the request: the check ends at the client timeout instead of hanging.
        let api = fake_api(|_, _, _| Reply::Hang).await;
        assert_eq!(check_at(&api.base, Duration::from_millis(200)).await, Err(EngineError::Timeout));
    }
}

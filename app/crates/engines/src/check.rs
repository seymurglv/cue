//! API key checks via free probe requests: the Anthropic model list, the OpenRouter key info and the account's credits,
//! the DeepSeek balance, an OpenAI-compatible service's model list (`chat::check`). Where the service tells, a key that
//! works on an account that cannot pay is `NoBalance`, not a working key: the cleanup would fail with it.
use serde_json::Value;

use crate::error::{checked, net, read_json, EngineError};

const ANTHROPIC_MODELS: &str = "https://api.anthropic.com/v1/models";
/// Key info (limit, spend) — free, no model request.
const OPENROUTER_KEY: &str = "https://openrouter.ai/api/v1/key";
/// The account's credits bought and used — free.
const OPENROUTER_CREDITS: &str = "https://openrouter.ai/api/v1/credits";
/// The account's balance and whether it can pay — free.
const DEEPSEEK_BALANCE: &str = "https://api.deepseek.com/user/balance";

/// The request sent with the key `key`: a success is the response, a refusal an error by status, a client timeout
/// `Timeout`. The key never appears in the error text, even if the service echoed it back.
async fn sent(req: reqwest::RequestBuilder, key: &str) -> Result<reqwest::Response, EngineError> {
    let res = match req.send().await {
        Ok(resp) => checked(resp, key).await,
        Err(e) if e.is_timeout() => Err(EngineError::Timeout),
        Err(e) => Err(net(e)),
    };
    res.map_err(|e| e.redact(key))
}

/// Probe request with the key `key`: a success means the key works (see [`sent`]).
async fn probe(req: reqwest::RequestBuilder, key: &str) -> Result<(), EngineError> {
    sent(req, key).await.map(drop)
}

/// A number the service gave as a number or as text (DeepSeek's amounts are strings).
fn amount(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
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

/// OpenRouter key: info about the key itself (a spent limit of the key is `NoBalance`), then the account's credits
/// (none left is `NoBalance`). Credits a key may not read leave the key's own answer.
pub async fn openrouter(http: &reqwest::Client, key: &str) -> Result<(), EngineError> {
    openrouter_at(http, OPENROUTER_KEY, OPENROUTER_CREDITS, key).await
}

pub(crate) async fn openrouter_at(
    http: &reqwest::Client,
    key_url: &str,
    credits_url: &str,
    key: &str,
) -> Result<(), EngineError> {
    let info: Value = read_json(sent(openrouter_request(http, key_url, key), key).await?).await?;
    if amount(info.pointer("/data/limit_remaining")).is_some_and(|left| left <= 0.0) {
        return Err(EngineError::NoBalance("the OpenRouter key's credit limit is spent".into()));
    }
    let Ok(resp) = sent(openrouter_request(http, credits_url, key), key).await else { return Ok(()) };
    let Ok(credits) = read_json::<Value>(resp).await else { return Ok(()) };
    let bought = amount(credits.pointer("/data/total_credits"));
    let used = amount(credits.pointer("/data/total_usage"));
    if let (Some(bought), Some(used)) = (bought, used)
        && bought - used <= 0.0
    {
        return Err(EngineError::NoBalance("no credits left on the OpenRouter account".into()));
    }
    Ok(())
}

/// DeepSeek key: the account's balance. An account that cannot pay is `NoBalance`; a refused key, a key problem.
pub async fn deepseek(http: &reqwest::Client, key: &str) -> Result<(), EngineError> {
    deepseek_at(http, DEEPSEEK_BALANCE, key).await
}

pub(crate) async fn deepseek_at(http: &reqwest::Client, url: &str, key: &str) -> Result<(), EngineError> {
    let balance: Value = read_json(sent(http.get(url).bearer_auth(key), key).await?).await?;
    match balance.get("is_available").and_then(Value::as_bool) {
        Some(false) => Err(EngineError::NoBalance("the DeepSeek account cannot pay".into())),
        _ => Ok(()),
    }
}

fn openrouter_request(http: &reqwest::Client, url: &str, key: &str) -> reqwest::RequestBuilder {
    http.get(url).bearer_auth(key)
}

/// An OpenAI-compatible service's model list at `url`, with the key as a bearer token if there is one (a local server
/// needs none).
pub(crate) async fn models_list(http: &reqwest::Client, url: &str, key: Option<&str>) -> Result<(), EngineError> {
    let req = http.get(url);
    match key {
        Some(key) => probe(req.bearer_auth(key), key).await,
        None => probe(req, "").await,
    }
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

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().timeout(LONG).build().unwrap()
    }

    #[tokio::test]
    async fn a_deepseek_account_that_can_pay_works() {
        let api = fake_api(|_, _, _| reply(200, r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"1.50"}]}"#)).await;
        assert_eq!(deepseek_at(&client(), &format!("{}/user/balance", api.base), KEY).await, Ok(()));
        assert_eq!(api.calls(), ["GET /user/balance"], "a free read of the balance, not a model request");
        assert_eq!(api.seen()[0].header("authorization"), [format!("Bearer {KEY}").as_str()]);
        assert_eq!(DEEPSEEK_BALANCE, "https://api.deepseek.com/user/balance");
    }

    #[tokio::test]
    async fn a_deepseek_account_with_no_money_is_no_balance_not_a_bad_key() {
        let api = fake_api(|_, _, _| reply(200, r#"{"is_available":false,"balance_infos":[{"currency":"CNY","total_balance":"0.00"}]}"#)).await;
        let e = deepseek_at(&client(), &api.base, KEY).await.unwrap_err();
        assert!(e.is_no_balance() && !e.is_key_problem(), "{e:?}");
    }

    #[tokio::test]
    async fn a_refused_deepseek_key_is_a_key_problem() {
        let api = fake_api(|_, _, _| reply(401, r#"{"error":{"message":"Authentication Fails"}}"#)).await;
        assert!(deepseek_at(&client(), &api.base, KEY).await.unwrap_err().is_key_problem());
    }

    /// OpenRouter at a fake service: the key info and the account's credits answer as given.
    async fn openrouter_with(key_info: &'static str, credits: (u16, &'static str)) -> (Result<(), EngineError>, Vec<String>) {
        let api = fake_api(move |_, path, _| if path.ends_with("/credits") { reply(credits.0, credits.1) } else { reply(200, key_info) }).await;
        let r = openrouter_at(&client(), &format!("{}/api/v1/key", api.base), &format!("{}/api/v1/credits", api.base), KEY).await;
        (r, api.calls())
    }

    #[tokio::test]
    async fn an_openrouter_key_with_credits_left_works() {
        let (r, calls) = openrouter_with(r#"{"data":{"limit":null,"limit_remaining":null}}"#, (200, r#"{"data":{"total_credits":10,"total_usage":2.5}}"#)).await;
        assert_eq!(r, Ok(()));
        assert_eq!(calls, ["GET /api/v1/key", "GET /api/v1/credits"]);
        assert_eq!(OPENROUTER_CREDITS, "https://openrouter.ai/api/v1/credits");
    }

    #[tokio::test]
    async fn an_openrouter_key_whose_limit_is_spent_is_no_balance() {
        let (r, _) = openrouter_with(r#"{"data":{"limit":5,"limit_remaining":0}}"#, (200, r#"{"data":{"total_credits":10,"total_usage":1}}"#)).await;
        assert!(r.unwrap_err().is_no_balance());
    }

    #[tokio::test]
    async fn an_openrouter_account_with_no_credits_left_is_no_balance() {
        let (r, _) = openrouter_with(r#"{"data":{"limit":null}}"#, (200, r#"{"data":{"total_credits":10,"total_usage":10}}"#)).await;
        assert!(r.unwrap_err().is_no_balance());
    }

    #[tokio::test]
    async fn credits_an_openrouter_key_may_not_read_leave_the_key_working() {
        let (r, _) = openrouter_with(r#"{"data":{"limit":null}}"#, (403, r#"{"error":{"message":"provisioning key required"}}"#)).await;
        assert_eq!(r, Ok(()));
    }

    #[tokio::test]
    async fn silent_service_is_timeout() {
        // The network "ate" the request: the check ends at the client timeout instead of hanging.
        let api = fake_api(|_, _, _| Reply::Hang).await;
        assert_eq!(check_at(&api.base, Duration::from_millis(200)).await, Err(EngineError::Timeout));
    }
}

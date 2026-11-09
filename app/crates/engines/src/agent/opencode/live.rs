//! The check against the OpenCode server running on this computer, through CUE's own code (first run 09.11.2026,
//! OpenCode Desktop 2.2.5). Ignored: it needs OpenCode running and a provider signed in, so it is run by hand:
//!
//! `cargo test -p cue-engines --lib agent::opencode::live -- --ignored --nocapture --test-threads=1`
//!
//! It sends only a harmless sentence, prints no password, header or token, and changes none of OpenCode's settings,
//! sessions or files: it checks that no session or project of CUE's is left behind, and that CUE's folder stays empty.
use std::sync::Arc;

use cue_core::agents::AgentId;
use cue_core::refine::RefineContext;
use cue_core::settings::{DictionarySettings, RefineSettings};
use reqwest::Method;
use serde_json::Value;

use super::*;
use crate::agent::{AgentClient, empty_cwd, refine};

const RAW: &str = "ну эм завтра в десять созвон с командой";

/// About 150 words of a harmless dictation (a weekend in the park), fillers and all.
const LONG: &str = "ну короче в эти выходные мы с друзьями решили съездить в парк за городом потому что погода обещает быть \
хорошей и солнечной эм сначала мы хотим встретиться у метро в десять утра купить в магазине воды хлеба сыра и фруктов а потом \
сесть на электричку которая идет примерно сорок минут ну и уже в парке найти тихое место у озера где можно постелить плед и \
спокойно посидеть в тени под деревьями кто-то обещал взять бадминтон и мяч так что поиграем немного а еще можно взять \
фотоаппарат и пофотографировать уток и цветы на клумбах там вроде как раз сейчас цветут тюльпаны вот после обеда мы планируем \
пройтись по большой тропинке вокруг озера она занимает где-то час и там есть смотровая площадка с красивым видом на лес а \
вечером часов в шесть хотим вернуться обратно в город чтобы не попасть в пробки и успеть отдохнуть перед рабочей неделей \
если вдруг пойдет дождь то просто зайдем в кафе у входа в парк и выпьем горячего чая с пирогами";

fn ids(v: &Value) -> Vec<String> {
    let list = v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter().filter_map(|s| s.get("id").and_then(Value::as_str).map(str::to_owned)).collect()
}

async fn get(s: &Server, path: &str) -> (StatusCode, Value) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let (status, body) = s.at.call(Method::GET, path, None, deadline).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "needs the user's running OpenCode server and a provider signed in"]
async fn live_opencode_cleans_through_cues_code() {
    // Found as discovery finds it, and as the slot uses it.
    let state = state_dir().expect("OpenCode's state folder");
    let announced = running(&state).expect("no OpenCode server announced and running");
    println!("announced: {} version {:?}", announced.url, announced.version);
    // A fixed folder: were it ever named to OpenCode, it would be kept as one project, not one per run.
    let home = std::env::temp_dir().join("cue-live-opencode");
    let cwd = empty_cwd(&home, AgentId::OpenCode).unwrap();
    let server = Arc::new(Server::announced(&state, &cwd).expect("the registration"));
    println!("server: {server:?}");

    // The text-free probe, step by step, then as CUE runs it.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (status, body) = server.at.call(Method::GET, "/global/health", None, deadline).await.unwrap();
    let kind = if body.starts_with(b"<") { "html" } else { "other" };
    println!("GET /global/health -> {status} ({kind}, {} bytes)", body.len());
    let (status, info) = get(&server, "/api/info").await;
    let keys: Vec<&String> = info.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    println!("GET /api/info -> {status}, keys {keys:?}, version {}", info["version"]);
    let started = std::time::Instant::now();
    let shape = server.shape(deadline).await.unwrap();
    println!("line: {shape:?} ({:?})", started.elapsed());
    assert_eq!(shape, Shape::Generate(0), "2.2.5 is the V2 line");

    // What OpenCode holds before: sessions and projects.
    let (status, sessions) = get(&server, "/api/session?limit=500").await;
    println!("GET /api/session -> {status}, {} sessions", ids(&sessions).len());
    let (status, projects) = get(&server, "/api/project").await;
    let project_count = projects.as_array().map(Vec::len).or_else(|| Some(ids(&projects).len()));
    println!("GET /api/project -> {status}, {project_count:?} projects");

    // The models, the default first, and "Быстрая · авто".
    let started = std::time::Instant::now();
    let models = server.models().await.unwrap();
    println!("models: {} in {:?}; first (default): {:?}", models.len(), started.elapsed(), models.first());
    let (_, default) = get(&server, "/api/model/default").await;
    // No folder named: the server's own.
    println!("default's location: {}", default["location"]);
    let auto = fast_auto(&models).expect("a model").clone();
    println!("fast_auto: {auto:?}");

    // One raw generation, to see the answer's shape and status (harmless text, CUE's own prompt).
    let client = AgentClient::OpenCode(server.clone());
    let (ctx, dict, s) = (RefineContext::default(), DictionarySettings::default(), RefineSettings::default());
    let p = refine::prompt(&client, RAW, &ctx, &dict, crate::prompt::Job::Rewrite, "");
    let body = json!({ "prompt": format!("{}\n\n{}", p.system, p.user), "model": { "providerID": auto.id.split_once('/').unwrap().0, "id": auto.id.split_once('/').unwrap().1 } });
    let started = std::time::Instant::now();
    let deadline = Instant::now() + Duration::from_secs(30);
    let (status, answer) = server.at.call(Method::POST, GENERATE_PATHS[0], Some(&body), deadline).await.unwrap();
    let answer: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
    let keys: Vec<&String> = answer.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    let data: Vec<&String> = answer["data"].as_object().map(|o| o.keys().collect()).unwrap_or_default();
    println!("POST {} -> {status} in {:?}; keys {keys:?}, data keys {data:?}", GENERATE_PATHS[0], started.elapsed());
    println!("  text: {:?}", answer["data"]["text"].as_str());

    // No tools: the route takes only a prompt and a model, and the model is asked what it could call.
    let ask = json!({ "prompt": "List the names of the tools or functions you can call in this conversation, one per line, or answer NONE if there are none.", "model": body["model"] });
    let (status, tools) = server.at.call(Method::POST, GENERATE_PATHS[0], Some(&ask), deadline).await.unwrap();
    let tools: Value = serde_json::from_slice(&tools).unwrap_or(Value::Null);
    println!("tools asked -> {status}: {:?}", tools["data"]["text"].as_str());

    // Three cleanups through CUE's real path, as a dictation runs them: the first with the start allowance.
    for run in 0..3 {
        let started = std::time::Instant::now();
        let r = refine::refine(&client, &auto.id, RAW, &ctx, &dict, &s, "", run == 0).await;
        println!("cleanup {} ({}): {:?} {:?} in {:?}", run + 1, auto.id, r.outcome, r.text, started.elapsed());
    }
    // And one with OpenCode's own default model.
    let started = std::time::Instant::now();
    let r = refine::refine(&client, "", RAW, &ctx, &dict, &s, "", true).await;
    println!("cleanup (default model): {:?} {:?} in {:?}", r.outcome, r.text, started.elapsed());
    println!("route used: {:?}", Shape::of(server.shape.load(Ordering::Relaxed)));

    // Nothing left behind: no new session, no new project, CUE's folder empty.
    let (_, after) = get(&server, "/api/session?limit=500").await;
    let new: Vec<String> = ids(&after).into_iter().filter(|id| !ids(&sessions).contains(id)).collect();
    println!("sessions after: {}, new: {new:?}", ids(&after).len());
    let (_, projects_after) = get(&server, "/api/project").await;
    let after_count = projects_after.as_array().map(Vec::len).or_else(|| Some(ids(&projects_after).len()));
    println!("projects after: {after_count:?}");
    let left: Vec<_> = std::fs::read_dir(&cwd).unwrap().flatten().map(|e| e.file_name()).collect();
    println!("CUE's folder after: {left:?}");
    assert!(new.is_empty(), "a session was left behind");
    assert_eq!(project_count, after_count, "a project was added");
    assert!(left.is_empty(), "something was written into CUE's folder");
}

/// What CUE finds on this computer, and a server CUE starts itself from what it finds (as it does when OpenCode
/// Desktop is not running), with OpenCode's own folders kept apart in a temporary folder so the user's settings,
/// sessions and registration are never touched; the user's running server is left alone.
#[tokio::test]
#[ignore = "needs OpenCode installed on this computer"]
async fn live_opencode_server_cue_starts() {
    let found = crate::agent::discover();
    println!("discover: {found:?}");
    let exe = crate::agent::discover::program(AgentId::OpenCode).expect("OpenCode's program");
    println!("program: {}", exe.display());
    // The user's registration: its size and time before, never its content.
    let state = state_dir().unwrap();
    let meta = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
    let registered = meta(&state.join("service.json"));

    let base = std::env::temp_dir().join("cue-live-opencode-serve");
    let apart = base.join("opencode-folders");
    let (data, config, cache, state_apart) = (apart.join("data"), apart.join("config"), apart.join("cache"), apart.join("state"));
    for d in [&data, &config, &cache, &state_apart] {
        std::fs::create_dir_all(d).unwrap();
    }
    let cwd = server_cwd(&base.join("agent")).unwrap();
    let env = [
        ("XDG_DATA_HOME", data.to_str().unwrap()),
        ("XDG_CONFIG_HOME", config.to_str().unwrap()),
        ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ("XDG_STATE_HOME", state_apart.to_str().unwrap()),
    ];
    let started = std::time::Instant::now();
    let server = serve(&exe, &cwd, &env).expect("opencode serve");
    println!("started in {:?}: {server:?}, pid {:?}", started.elapsed(), server.pid());

    // Without the password it is refused; with CUE's, it answers as V2.
    let stranger = Server::running_at(server.url.clone(), None, None);
    let deadline = Instant::now() + Duration::from_secs(20);
    let refused = stranger.at.call(Method::GET, "/api/info", None, deadline).await;
    println!("GET /api/info without the password -> {refused:?}");
    let shape = server.shape(deadline).await;
    println!("line: {shape:?}");
    let models = server.models().await.map(|m| m.len());
    println!("models: {models:?}");
    let in_cwd = |label: &str| {
        let left: Vec<_> = std::fs::read_dir(&cwd).unwrap().flatten().map(|e| e.file_name()).collect();
        println!("CUE's folder {label}: {left:?}");
        left
    };
    let (_, projects) = get(&server, "/api/project").await;
    let canonical: Vec<&str> = projects.as_array().into_iter().flatten().filter_map(|p| p["canonical"].as_str()).collect();
    println!("its projects: {canonical:?}");
    let (_, default) = get(&server, "/api/model/default").await;
    println!("its default location: {}", default["location"]);
    let while_running = in_cwd("while it runs");
    let apart_state: Vec<_> = std::fs::read_dir(&state_apart).unwrap().flatten().map(|e| e.path()).collect();
    println!("its state folder: {apart_state:?}");
    let mut apart_top: Vec<_> = Vec::new();
    for d in [&data, &config, &cache] {
        apart_top.extend(std::fs::read_dir(d).unwrap().flatten().map(|e| e.path()));
    }
    println!("its other folders: {apart_top:?}");
    server.stop();
    let after = in_cwd("after it stopped");
    let still = meta(&state.join("service.json"));
    println!("the user's registration unchanged: {}", still == registered);
    assert_eq!(refused, Err(AgentFailure::Failed("auth")));
    assert_eq!(shape, Ok(Shape::Generate(0)));
    assert!(while_running.is_empty() && after.is_empty(), "the server wrote into CUE's folder");
    assert_eq!(still, registered, "the user's registration changed");
}

/// A long dictation (about 150 words) cleaned three times through CUE's real path, against its budget.
#[tokio::test]
#[ignore = "needs the user's running OpenCode server and a provider signed in"]
async fn live_opencode_long_dictation() {
    let state = state_dir().expect("OpenCode's state folder");
    let home = std::env::temp_dir().join("cue-live-opencode");
    let cwd = empty_cwd(&home, AgentId::OpenCode).unwrap();
    let server = Arc::new(Server::announced(&state, &cwd).expect("the registration"));
    let auto = fast_auto(&server.models().await.unwrap()).expect("a model").clone();
    let client = AgentClient::OpenCode(server);
    let (ctx, dict, s) = (RefineContext::default(), DictionarySettings::default(), RefineSettings::default());
    let words = LONG.split_whitespace().count();
    let budget = refine::timeout_ms(&client, LONG, false);
    let rules_ms = cue_core::refine::timeout_ms(LONG);
    println!("{words} words, {} chars; refine::timeout_ms {rules_ms} ms (70%: {} ms); budget now {budget} ms; model {}", LONG.chars().count(), rules_ms * 7 / 10, auto.id);
    for run in 0..3 {
        // Within the budget a dictation has (not the first request), each step as `refine::run` takes it.
        let p = refine::prompt(&client, LONG, &ctx, &dict, crate::prompt::Job::Rewrite, "");
        let started = std::time::Instant::now();
        let answer = client.complete(&auto.id, &p, Duration::from_millis(budget), None).await;
        let took = started.elapsed();
        let Ok(answer) = answer else {
            println!("long cleanup {}: {answer:?} in {took:?}", run + 1);
            continue;
        };
        let parsed = crate::prompt::parse_content(&answer, None, false);
        let form = if answer.trim_start().starts_with('{') { "json" } else if answer.trim_start().starts_with("```") { "fenced" } else { "plain" };
        let (outcome, reason) = match &parsed {
            crate::prompt::Parsed::Text(t) if cue_core::refine::opens_with_lead_in(LONG, t, &[], &[]) => {
                (cue_core::flow::RefineOutcome::RawFallback, "the answer opens with a lead-in")
            }
            crate::prompt::Parsed::Text(t) => {
                let tidy = crate::prompt::decide(LONG, crate::prompt::Parsed::Text(t.clone()), &dict, &s);
                let ratio = t.chars().count() as f32 / LONG.chars().count() as f32;
                let why = if tidy.1 == cue_core::flow::RefineOutcome::Refined { "" } else { "failed the sanity check" };
                println!("  answer {} chars, {ratio:.2} of what was said", t.chars().count());
                (tidy.1, why)
            }
            other => (cue_core::flow::RefineOutcome::RawFallback, if *other == crate::prompt::Parsed::Malformed { "does not match the schema" } else { "refused or cut" }),
        };
        // The reason on every run that is not a cleanup; never the text.
        println!("long cleanup {}: {outcome:?} {reason} ({form}) in {took:?}", run + 1);
    }
}

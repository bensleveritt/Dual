#![feature(proc_macro_hygiene, decl_macro)]
extern crate anyhow;

use rust_bert::gpt2::{
    GPT2Generator, Gpt2ConfigResources, Gpt2MergesResources, Gpt2ModelResources, Gpt2VocabResources,
};
use rust_bert::pipelines::common::{ModelType, TokenizerOption};
use rust_bert::pipelines::generation_utils::{GenerateConfig, LanguageGenerator};
use rust_bert::resources::{RemoteResource, Resource};

use std::ops::Deref;
use tch::Tensor;

use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;
use tokio::task;
use warp::Filter;

type TextGenModel = Arc<Mutex<GPT2Generator>>;
type TextGenTokenizer = Arc<Mutex<TokenizerOption>>;

fn textgen_model_config() -> GenerateConfig {
    let config = GenerateConfig {
        max_length: 200,
        model_resource: Resource::Remote(RemoteResource::from_pretrained(Gpt2ModelResources::GPT2)),
        config_resource: Resource::Remote(RemoteResource::from_pretrained(
            Gpt2ConfigResources::GPT2,
        )),
        vocab_resource: Resource::Remote(RemoteResource::from_pretrained(Gpt2VocabResources::GPT2)),
        merges_resource: Resource::Remote(RemoteResource::from_pretrained(
            Gpt2MergesResources::GPT2,
        )),
        do_sample: false,
        num_beams: 5,
        ..Default::default()
    };
    config
}

fn textgen_model(config: GenerateConfig) -> TextGenModel {
    let textgen_model = GPT2Generator::new(config).expect("Model failed to load");
    Arc::new(Mutex::new(textgen_model))
}

fn textgen_tokenizer(config: GenerateConfig) -> TextGenTokenizer {
    let vocab_path = config.vocab_resource.get_local_path().expect("Failed");
    let merges_path = config.merges_resource.get_local_path().expect("Failed");

    let textgen_tokenizer = TokenizerOption::from_file(
        ModelType::GPT2,
        vocab_path.to_str().unwrap(),
        Some(merges_path.to_str().unwrap()),
        false,
        None,
        None,
    )
    .unwrap();
    Arc::new(Mutex::new(textgen_tokenizer))
}

async fn generate(
    query: TextGenQuery,
    textgen_model: TextGenModel,
    textgen_tokenizer: TextGenTokenizer,
) -> Result<impl warp::Reply, Infallible> {
    let model = textgen_model.lock().await;
    let tokenizer = textgen_tokenizer.lock().await;

    let allowed_tokens =
        allowed_tokens_factory(string_to_static_str(query.prompt.clone()), &tokenizer, 3, 1);

    let output = model.generate(
        Some(&[string_to_static_str(query.prompt.clone())]),
        None,
        None,
        None,
        None,
        Some(allowed_tokens.deref()),
    );

    let prompt_len = query.prompt.clone().len();
    let output_sliced = (&output[0][prompt_len..]).to_string();

    let mut response = HashMap::new();
    response.insert("output", output_sliced);

    Ok(warp::reply::json(&response))
}

fn string_to_static_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn allowed_tokens_factory<'a>(
    prompt: &'a str,
    tokenizer: &'a MutexGuard<TokenizerOption>,
    generated_sentences: usize,
    generated_paragraphs: usize,
) -> Box<dyn Fn(i64, &Tensor) -> Vec<i64> + 'a> {
    Box::new(move |_batch_id: i64, previous_token_ids: &Tensor| {
        let previous_token_ids_vec: Vec<i64> = previous_token_ids.into();
        let tokenized_prompt = tokenizer.tokenize(prompt);
        let generated_tokens = &previous_token_ids_vec[tokenized_prompt.len()..];

        let sentence_token_count: usize = generated_tokens
            .iter()
            .filter(|&n| *n == 13 || *n == 30 || *n == 0)
            .count();
        let paragraph_token_count: usize = generated_tokens
            .iter()
            .filter(|&n| *n == 198 || *n == 628)
            .count();

        if sentence_token_count == generated_sentences
            || paragraph_token_count == generated_paragraphs
        {
            return vec![50256];
        }

        (0..50255).collect()
    })
}

#[derive(Debug, Deserialize)]
pub struct TextGenQuery {
    pub prompt: String,
}

fn with_model(
    textgen_model: TextGenModel,
) -> impl Filter<Extract = (TextGenModel,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || textgen_model.clone())
}

fn with_tokenizer(
    textgen_tokenizer: TextGenTokenizer,
) -> impl Filter<Extract = (TextGenTokenizer,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || textgen_tokenizer.clone())
}

fn json_body() -> impl Filter<Extract = (TextGenQuery,), Error = warp::Rejection> + Clone {
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

#[tokio::main]
async fn main() {
    let textgen_tokenizer: TextGenTokenizer = task::spawn_blocking(move || {
        let c = textgen_model_config();
        let t = textgen_tokenizer(c);
        t
    })
    .await
    .expect("Working");

    let textgen_model: TextGenModel = task::spawn_blocking(move || {
        let c = textgen_model_config();
        let m = textgen_model(c);
        m
    })
    .await
    .expect("Working");

    println!("Loaded config and model");

    let textgen_handler = warp::path!("generate")
        .and(warp::post())
        .and(json_body())
        .and(with_model(textgen_model.clone()))
        .and(with_tokenizer(textgen_tokenizer.clone()))
        .and_then(generate);

    println!("Starting to serve...");
    warp::serve(textgen_handler)
        .run(([127, 0, 0, 1], 3030))
        .await;
}
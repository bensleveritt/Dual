use crate::server::Query;
use crate::utils::*;

use rust_bert::gpt2::{
    GPT2Generator, Gpt2ConfigResources, Gpt2MergesResources, Gpt2ModelResources, Gpt2VocabResources,
};
use rust_bert::pipelines::common::{ModelType, TokenizerOption};
use rust_bert::pipelines::generation_utils::{GenerateConfig, LanguageGenerator};
use rust_bert::resources::{RemoteResource, Resource};
use tch::Tensor;

use regex::Regex;
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;

pub type TextGenModel = Arc<Mutex<GPT2Generator>>;
pub type TextGenTokenizer = Arc<Mutex<TokenizerOption>>;

/// Initialize model config
pub fn textgen_model_config() -> GenerateConfig {
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
        num_return_sequences: 5,
        ..Default::default()
    };
    config
}

/// Load model
pub fn textgen_model(config: GenerateConfig) -> TextGenModel {
    let textgen_model = GPT2Generator::new(config).expect("Model failed to load");
    Arc::new(Mutex::new(textgen_model))
}

/// Load tokenizer
pub fn textgen_tokenizer(config: GenerateConfig) -> TextGenTokenizer {
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

/// Generate completions
pub async fn generate(
    query: Query,
    textgen_model: TextGenModel,
    textgen_tokenizer: TextGenTokenizer,
) -> Vec<String> {
    let model = textgen_model.lock().await;
    let tokenizer = textgen_tokenizer.lock().await;
    let prompt = query.prompt.clone();
    let prompt_len = prompt.len();

    let context_tokens: Vec<Vec<String>>;
    let context_ids: Option<Vec<Vec<i64>>>;

    if let Some(context) = query.context.clone() {
        context_tokens = context
            .iter()
            .map(|e| tokenizer.tokenize(e.clone().as_str()))
            .collect();
        context_ids = Some(
            context_tokens
                .iter()
                .map(|e| tokenizer.convert_tokens_to_ids(e))
                .collect(),
        );
    } else {
        context_ids = None;
    }

    let allowed_tokens = allowed_tokens_factory(
        prompt.as_str(),
        &tokenizer,
        query.generate_sentences.clone(),
        query.generate_paragraphs.clone(),
        context_ids,
    );

    let output = model.generate(
        Some(&[query.prompt.clone().as_str()]),
        None,
        None,
        None,
        None,
        Some(allowed_tokens.deref()),
    );

    output
        .iter()
        .map(|e| (&e[prompt_len..]).to_string())
        .collect()
}

/// Specify allowed tokens at each generation step
fn allowed_tokens_factory<'a>(
    prompt: &'a str,
    tokenizer: &'a MutexGuard<TokenizerOption>,
    generated_sentences: Option<usize>,
    generated_paragraphs: Option<usize>,
    context_ids: Option<Vec<Vec<i64>>>,
) -> Box<dyn Fn(i64, &Tensor) -> Vec<i64> + 'a> {
    Box::new(move |_batch_id: i64, previous_token_ids: &Tensor| {
        let previous_token_ids_vec: Vec<i64> = previous_token_ids.into();
        let tokenized_prompt = tokenizer.tokenize(prompt);
        let generated_ids = &previous_token_ids_vec[tokenized_prompt.len()..];

        let generated_text = tokenizer.decode(generated_ids.into(), true, true);
        let re = Regex::new(
            r"([a-zA-Z0-9]?\.[a-zA-Z0-9]*\.|[0-9]+\.[0-9]*|[A-Z]+[a-z]*\.\s[a-zA-Z]{1})",
        )
        .unwrap();
        let clean_generated_text = re.replace_all(generated_text.as_str(), "");
        let clean_generated_tokens = tokenizer.tokenize(&clean_generated_text);
        let clean_generated_ids = tokenizer.convert_tokens_to_ids(clean_generated_tokens);

        let sentence_token_count: usize = clean_generated_ids
            .iter()
            .filter(|&n| *n == 13 || *n == 30 || *n == 0)
            .count();
        let paragraph_token_count: usize = clean_generated_ids
            .iter()
            .filter(|&n| *n == 198 || *n == 628)
            .count();

        if let Some(gs) = generated_sentences {
            if sentence_token_count == gs {
                return vec![50256];
            }
        }

        if let Some(gp) = generated_paragraphs {
            if paragraph_token_count == gp {
                return vec![50256];
            }
        }

        if let Some(c) = &context_ids {
            if generated_ids.len() == 0 {
                return c.iter().fold(vec![], |mut a, b| {
                    a.append(&mut b.clone());
                    a
                });
            }

            let allowed_token_ids: Vec<Vec<i64>> = c
                .iter()
                .map(|e| {
                    let mut local_context_ids = e.clone();
                    let mut local_candidate_ids: Vec<i64> = vec![];

                    while let Some(start) = find_subsequence(&local_context_ids, &generated_ids) {
                        let end = start + generated_ids.len();
                        if end < local_context_ids.len() {
                            local_candidate_ids.push(local_context_ids[end]);
                        }
                        local_context_ids = local_context_ids[end..].into();
                    }
                    local_candidate_ids
                })
                .collect();

            let mut all_allowed_token_ids = allowed_token_ids.iter().fold(vec![], |mut a, b| {
                a.append(&mut b.clone());
                a
            });

            all_allowed_token_ids.append(&mut vec![50256]);
            return all_allowed_token_ids;
        }

        (0..50255).collect()
    })
}
use std::{fs, iter::zip};
use regex::Regex;
use itertools::Itertools;

use actix_web::{get, web, App, error::ErrorBadRequest, HttpServer, Responder, Result};

struct AppState {
    corpus: Vec<(usize, Vec<String>)>,
    most_common: Vec<(usize, Vec<(char, usize)>)>,
}

fn get_words<T>(corpus: &[(usize, T)], length: usize) -> Option<&T> {
    corpus.iter()
        .find(|(l, _)| l == &length)
        .map(|(_, w)| w)
}

#[derive(Debug, Clone)]
enum WordCluePattern {
    Letter(char),
    Exclude(Vec<char>),
}

#[derive(Debug, Copy, Clone)]
struct WordClueLetter {
    letter: char,
    count: usize,
    exact: bool,
}

#[derive(Debug, Clone)]
struct WordClue {
    pattern: Vec<WordCluePattern>,
    letters: Vec<WordClueLetter>,
}

fn merge_letter_clue(map: &mut Vec<WordClueLetter>, clue: WordClueLetter) {
    if let Some((idx, w)) = map.iter().enumerate().find(|(_, WordClueLetter{ letter: l, ..})| *l == clue.letter) {
        map[idx] = WordClueLetter {
            letter: w.letter,
            count: (w.count).max(clue.count),
            exact: w.exact || clue.exact,
        };
    } else {
        map.push(clue);
    }
}

fn merge(a: WordClue, b: WordClue) -> Result<WordClue, String> {
    Ok(WordClue {
        pattern: {
            if a.pattern.len() != b.pattern.len() {
                return Err(format!("Pattern length mismatch: {} != {}", a.pattern.len(), b.pattern.len()));
            }
            zip(a.pattern.into_iter(), b.pattern.into_iter()).map(|(a, b)|{
                match (a, b) {
                    (WordCluePattern::Letter(a), WordCluePattern::Letter(b)) => {
                        if a == b {
                            Ok(WordCluePattern::Letter(a))
                        } else {
                            Err(format!("Conflict: {} != {}", a, b))
                        }
                    },
                    (WordCluePattern::Letter(a), _) => Ok(WordCluePattern::Letter(a)),
                    (_, WordCluePattern::Letter(b)) => Ok(WordCluePattern::Letter(b)),
                    (WordCluePattern::Exclude(a), WordCluePattern::Exclude(b)) => {
                        Ok(WordCluePattern::Exclude(a.into_iter().chain(b.into_iter()).sorted().dedup().collect()))
                    },
                }
            }).collect::<Result<Vec<_>,_>>()?
        },
        letters: {
            let mut a_vec = a.letters;
            for w in b.letters {
                merge_letter_clue(&mut a_vec, w);
            }
            a_vec
        },
    })
}

enum LetterAnswerType {
    Correct,
    Incorrect,
    NotInWord,
}

struct LetterAnswer {
    letter: char,
    answer: LetterAnswerType,
}

type WordAnswer = Vec<LetterAnswer>;

fn extract_clue(word: WordAnswer) -> WordClue {
    let mut pattern: Vec<WordCluePattern> = Vec::new();
    let mut letter_clues: Vec<WordClueLetter> = Vec::new();
    let mut exclude: Vec<char> = Vec::new();

    for LetterAnswer{letter, answer} in word.into_iter() {
        match answer {
            LetterAnswerType::Correct => {
                pattern.push(WordCluePattern::Letter(letter));
                match letter_clues.iter().enumerate().find(|(_, WordClueLetter{ letter: l, ..})| *l == letter) {
                    Some((idx, _)) => letter_clues[idx].count += 1,
                    None => letter_clues.push(WordClueLetter{ letter, count: 1, exact: false }),
                };
            },
            LetterAnswerType::Incorrect => {
                pattern.push(WordCluePattern::Exclude(vec![letter]));
                match letter_clues.iter().enumerate().find(|(_, WordClueLetter{ letter: l, ..})| *l == letter) {
                    Some((idx, _)) => letter_clues[idx].count += 1,
                    None => letter_clues.push(WordClueLetter{ letter, count: 1, exact: false }),
                };
            },
            LetterAnswerType::NotInWord => {
                pattern.push(WordCluePattern::Exclude(vec![letter]));
                exclude.push(letter);
            },
        }
    }

    let exclude = {
        let mut result = Vec::new();
        for e in exclude.into_iter() {
            match letter_clues.iter().enumerate().find(|(_, WordClueLetter{ letter: l, ..})| l == &e) {
                Some((idx, _)) => letter_clues[idx].exact = true,
                None => result.push(e),
            };
        }
        result
    };

    WordClue {
        pattern: pattern.into_iter().map(|p| match p {
            WordCluePattern::Exclude(v) => {
                WordCluePattern::Exclude(v.into_iter().chain(exclude.clone()).sorted().dedup().collect())
            },
            _ => p,
        }).collect(),
        letters: letter_clues,
    }

}

fn extract_answer(token: &str) -> Result<WordAnswer, String> {
    let pattern = Regex::new(r"([A-Za-zçÇ])([0-2])").unwrap();
    let check_pattern = Regex::new(r"^([A-Za-zçÇ][0-2])+$").unwrap();
    if !check_pattern.is_match(token) {
        return Err(format!("Invalid token: {}", token));
    }

    pattern.captures_iter(token).map(|c| {
        let (_, [letter, answer]) = c.extract();
        let letter = letter.to_uppercase().chars().next().unwrap();
        let answer = match answer.chars().next().unwrap() {
            '0' => LetterAnswerType::NotInWord,
            '1' => LetterAnswerType::Incorrect,
            '2' => LetterAnswerType::Correct,
            _ => return Err(format!("Invalid answer value: {}", answer)),
        };
        Ok(LetterAnswer{letter, answer})
    }).collect()
}

fn filter<'a, T: AsRef<str>>(clue: &WordClue, words: &'a [T]) -> Vec<&'a str> {

    let pattern = Regex::new(
        &clue.pattern.iter().map(|p| match p {
            WordCluePattern::Letter(l) => format!("{}", l),
            WordCluePattern::Exclude(v) => format!("[^{}]", v.iter().collect::<String>()),
        }).collect::<String>()
    ).unwrap();

    let letter_patterns = clue.letters.iter().map(|w| match w {
        WordClueLetter { letter: l, count: c, exact: true } => {
            Regex::new(format!(r"([^{}]*{}){{{}}}", l, l, c).as_str()).unwrap()
        },
        WordClueLetter { letter: l, count: c, exact: false } => {
            Regex::new(format!(r"([^{}]*{}){{{},}}", l, l, c).as_str()).unwrap()
        },
    }).collect::<Vec<_>>();

    words.iter().filter(|w| {
        let w = w.as_ref();
        pattern.is_match(w) && letter_patterns.iter().all(|p| p.is_match(w))
    }).map(|w| w.as_ref()).collect()
}

#[get("/api/words/{pattern:[/a-zA-ZçÇ0-2]+}")]
async fn api_words(path: web::Path<String>, state: web::Data<AppState>) -> Result<impl Responder> {
    let clue = {
        let mut clues : Vec<WordClue> = path.to_uppercase().split('/').map(|token|{
            extract_answer(token)
                .map(extract_clue)
                .map_err( ErrorBadRequest)
        }).collect::<Result<Vec<_>,_>>()?;

        let mut result = clues.pop().ok_or(ErrorBadRequest("Empty pattern"))?;
        for clue in clues.into_iter() {
            result = merge(result, clue).map_err(ErrorBadRequest)?;
        }
        result
    };

    Ok(web::Json(
        get_words(&state.corpus, clue.pattern.len())
            .map_or(vec![], |words| filter(&clue, words).into_iter().map(String::from).collect::<Vec<_>>())
    ))
}

fn get_frequency(word: &str) -> Vec<(char, usize)> {
    word.chars().sorted().group_by(|c| *c).into_iter().map(|(c, g)| (c, g.count())).collect()
}

fn score(expected: &[(char, usize)], frequency: &[(char, usize)]) -> usize {
    let mut score = 0;
    for (c, f) in expected.iter() {
        if let Some((_, e)) = frequency.iter().find(|(l, _)| l == c) {
            score += e.min(f);
        }
    }
    score
}

fn weighted_score(expected: &[(char, usize)], frequency: &[(char, usize)]) -> usize {
    let mut score = 0;
    for (c, f) in expected.iter() {
        if frequency.iter().any(|(l, _)| l == c) {
            score += f;
        }
    }
    score
}


#[get("/api/most_letters/{n}/{pattern:[a-zA-ZçÇ]+}")]
async fn api_most_letters(path: web::Path<(usize, String)>, state: web::Data<AppState>) -> Result<impl Responder> {
    let (n, pattern) = path.into_inner();
    let freq = get_frequency(pattern.to_uppercase().as_str());

    Ok(web::Json(
        get_words(&state.corpus, n).map(|ws| 
            ws.iter().map(|a| (a, score(&freq, &get_frequency(a))))
            .sorted_by_key(|(_, s)| *s).rev()
            .group_by(|(_, s)| *s).into_iter()
            .next()
            .map_or(vec!["".to_string()], |(_, grp)| grp.into_iter().map(|(w, _)| w.to_owned()).collect())
        )
        .unwrap_or(vec!["".to_string()])
    ))
}

#[get("/api/most_common/{n}")]
async fn api_most_common(path: web::Path<usize>, state: web::Data<AppState>) -> Result<impl Responder> {
    let n = path.into_inner();

    Ok(web::Json(
        zip(get_words(&state.corpus, n), get_words(&state.most_common, n))
            .map(|(ws, mc)| 
                ws.iter().map(|a| (a, weighted_score(mc, &get_frequency(a))))
                .sorted_by_key(|(_, s)| *s).rev()
                .group_by(|(_, s)| *s).into_iter()
                .next()
                .map_or(vec!["".to_string()], |(_, grp)| grp.into_iter().map(|(w, _)| w.to_owned()).collect())
            )
            .next().unwrap_or(vec!["".to_string()])
    ))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let corpus = fs::read_to_string("data/corpus.txt")
        .expect("Failed to read corpus.txt").lines()
        .sorted_by_key(|w| w.len())
        .group_by(|w| w.len()).into_iter()
        .map(|(l, w)| (l, w.into_iter().map(String::from).collect_vec()))
        .collect::<Vec<_>>();
    let most_common = corpus.iter().map(|(n, words)|{
        let mut freq = words.iter().map(|w| get_frequency(w)).fold(Vec::new(), |mut acc, f| {
            for (c, _) in f {
                match acc.iter().enumerate().find(|(_, (l, _))| l == &c) {
                    Some((idx, _)) => acc[idx].1 += 1,
                    None => acc.push((c, 1_usize)),
                }
            }
            acc
        });
        freq.sort_by_key(|(_, f)| *f);
        (*n, freq.into_iter().rev().collect::<Vec<_>>())
    }).collect::<Vec<_>>();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(AppState {
                corpus: corpus.clone(),
                most_common: most_common.clone(),
            }))
            .service(api_words)
            .service(api_most_letters)
            .service(api_most_common)
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
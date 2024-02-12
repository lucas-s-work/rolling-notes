use core::fmt;
use std::{
    fmt::{Display, Formatter},
    fs::File,
    io::{Read, Write},
    path::Path,
};

use anyhow::Result;
use chrono::NaiveDate;
use clap::{command, Args, Parser, ValueEnum};
use inquire::{Select, Text};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, ValueEnum)]
enum JotState {
    Completed,
    Removed,
    InProgess,
    Failed,
    NotStarted,
}

impl Display for JotState {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let fmt_str = match self {
            Self::Completed => "Completed",
            Self::Removed => "Removed",
            Self::InProgess => "In Progress",
            Self::Failed => "Failed",
            Self::NotStarted => "Not Started",
        };

        write!(f, "{}", fmt_str)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct Jot {
    value: String,
    state: JotState,
}

impl Display for Jot {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}: {}", &self.value, &self.state)
    }
}

impl Jot {
    fn is_terminal(&self) -> bool {
        match self.state {
            JotState::Completed | JotState::Removed | JotState::Failed => true,
            JotState::InProgess | JotState::NotStarted => false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
enum DateInterval {
    InProgress {
        start: chrono::NaiveDate,
    },
    Complete {
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    },
}

impl Display for DateInterval {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let format = match self {
            Self::InProgress { start } => format!("In Progress, start: {}", start),
            Self::Complete { start, end } => format!("Complete, start: {}, end {}", start, end),
        };

        write!(f, "{}", format)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct JotSet {
    jots: Vec<Jot>,
    interval: DateInterval,
}

impl Display for JotSet {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let title = self.interval.to_string();
        let formatted_jots = self
            .jots
            .iter()
            .map(Jot::to_string)
            .map(|j| format!("- {}", j))
            .collect::<Vec<_>>()
            .join("\n");
        write!(f, "Jots: {}\n{}", title, formatted_jots)
    }
}

impl Default for JotSet {
    fn default() -> Self {
        Self {
            jots: Vec::new(),
            interval: DateInterval::InProgress {
                start: chrono::Utc::now().date_naive(),
            },
        }
    }
}

impl JotSet {
    fn get_non_terminal_jots(&self) -> Vec<Jot> {
        self.jots
            .iter()
            .filter(|j| !j.is_terminal())
            .map(Jot::clone)
            .collect()
    }

    fn filter_by_states(&self, states: Vec<JotState>) -> JotSet {
        if states.is_empty() {
            self.clone()
        } else {
            let filtered_jots = self
                .jots
                .iter()
                .filter(|j| states.contains(&j.state))
                .map(Jot::clone)
                .collect();

            JotSet {
                jots: filtered_jots,
                interval: self.interval.clone(),
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct JotHistory {
    sets: Vec<JotSet>,
}

#[derive(Error, Debug)]
enum HistoryLoadError {
    #[error("failed to read config file contents")]
    FailedToReadConfigFile(#[from] std::io::Error),

    #[error("config file is malformed")]
    MalformedConfig(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
enum HistorySaveError {
    #[error("failed to serialize config: {0}")]
    FailedToSerializeConfig(#[from] serde_json::Error),

    #[error("failed to write config to file: {0}")]
    FailedToWriteConfig(#[from] std::io::Error),
}

impl Display for JotHistory {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let sets_str = self
            .sets
            .iter()
            .map(|j| format!("- {}", j.interval.to_string()))
            .collect::<Vec<_>>()
            .join("\n");
        write!(f, "{}", sets_str)
    }
}

impl JotHistory {
    fn load_from_file_or_create(path: &Path) -> Result<JotHistory, HistoryLoadError> {
        match File::open(path) {
            Ok(mut file) => {
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)?;
                Ok(serde_json::from_slice(&buf)?)
            }
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    File::create(path)?;
                    Ok(JotHistory {
                        sets: vec![JotSet::default()],
                    })
                }
                _ => Err(HistoryLoadError::FailedToReadConfigFile(err)),
            },
        }
    }

    fn save(&self, path: &Path) -> Result<(), HistorySaveError> {
        let mut file = File::options().truncate(true).write(true).open(path)?;
        let bytes = serde_json::to_vec(self)?;
        file.write_all(&bytes)?;
        Ok(())
    }

    /// Roll over the jotset, leaving behind any terminal jots and stamping the time
    fn roll(&mut self) {
        // get the last jotset from the note history and make a copy of any terminal notes
        // This vec should always contain atleast one element so this is a safe unwrap
        let mut last_jotset = self.sets.pop().unwrap();
        let rolled_jots = last_jotset.get_non_terminal_jots();
        let now = chrono::Utc::now().date_naive();

        last_jotset.interval = match last_jotset.interval {
            DateInterval::Complete { start, end } => DateInterval::Complete { start, end },
            DateInterval::InProgress { start } => DateInterval::Complete {
                start,
                end: now.clone(),
            },
        };
        self.sets.push(last_jotset);

        let new_jotset = JotSet {
            jots: rolled_jots,
            interval: DateInterval::InProgress { start: now },
        };
        self.sets.push(new_jotset);
    }

    fn get(&self) -> JotSet {
        // Same as above this last element must exist due to our initialization
        self.sets.last().unwrap().clone()
    }

    fn get_with_date(&self, date: NaiveDate) -> Option<JotSet> {
        self.sets
            .iter()
            .find(|set| match set.interval {
                DateInterval::InProgress { start } => date >= start,
                DateInterval::Complete { start, end } => date >= start && date <= end,
            })
            .map(JotSet::clone)
    }

    fn insert(&mut self, jot: Jot) {
        // Same as above this last element must exist due to our initialization
        self.sets.last_mut().unwrap().jots.push(jot);
    }

    fn set_jot(&mut self, jot: Jot, index: usize) {
        let mut jot_set = self.sets.pop().unwrap();
        jot_set.jots[index] = jot;
        self.sets.push(jot_set);
    }

    /// overwrite the current (last element) jotset with another
    fn set(&mut self, set: JotSet) {
        self.sets.pop();
        self.sets.push(set);
    }

    fn get_date_intervals(&self) -> Vec<DateInterval> {
        self.sets.iter().map(|s| s.interval.clone()).collect()
    }
}

fn main() -> Result<()> {
    let path = Path::new(".history.json");
    let mut history =
        JotHistory::load_from_file_or_create(&path).expect("failed to create or load jot history");

    match Commands::parse() {
        Commands::View(args) => view(&history, args),
        Commands::ViewHistory => view_history(&history)?,
        Commands::New(args) => new(&mut history, args)?,
        Commands::Update(args) => update(&mut history, args)?,
        Commands::Delete => (),
        Commands::Roll => roll(&mut history),
    };

    history.save(path)?;
    Ok(())
}

#[derive(Args, Debug)]
struct ViewArgs {
    #[arg(short, long)]
    date: Option<NaiveDate>,
    #[arg(short, long)]
    states: Vec<JotState>,
}

#[derive(Args, Debug)]
struct NewArgs {
    #[arg(short, long)]
    jot: Option<String>,

    #[arg(short, long)]
    state: Option<JotState>,
}

type UpdateArgs = NewArgs;

#[derive(Parser, Debug)]
enum Commands {
    /// View the jots in the current set, if a date is specified view the set which contains that
    /// date  
    View(ViewArgs),

    /// Open a selector for the sets by dates and then view the set
    ViewHistory,

    /// Create a new Jot and place into the current set, also the default
    /// Omitting any arguments will enter interactive mode for generating the jot
    New(NewArgs),

    /// Update a jot in the set, allows selecting the jot
    Update(UpdateArgs),

    /// Choose a selection of jots to delete from the in progress set
    Delete,

    /// Roll over unfinished jots into a new set, display the new set
    Roll,
}

fn view(history: &JotHistory, args: ViewArgs) {
    if let Some(date) = args.date {
        match history.get_with_date(date) {
            None => println!("No jotset found for date"),
            Some(set) => println!("{}", set.filter_by_states(args.states)),
        }
    } else {
        let filtered_jots = history.get().filter_by_states(args.states);
        println!("{}", filtered_jots)
    }
}

fn roll(history: &mut JotHistory) {
    history.roll();
    println!("{}", history.get());
}

fn prompt_state() -> Result<JotState> {
        let states = vec![
            JotState::Completed,
            JotState::Removed,
            JotState::InProgess,
            JotState::Failed,
            JotState::NotStarted,
        ];
        Ok(Select::new("enter jot state", states)
            .prompt_skippable()?
            .unwrap_or(JotState::NotStarted))
}

fn new(history: &mut JotHistory, args: NewArgs) -> Result<()> {
    let message = if let Some(message) = args.jot {
        message
    } else {
        Text::new("Enter jot").prompt()?
    };

    let state = if let Some(state) = args.state {
        state
    } else {
        prompt_state()?
    };

    history.insert(Jot {
        value: message,
        state,
    });

    Ok(())
}

fn update(history: &mut JotHistory, args: UpdateArgs) -> Result<()> {
    let set = history.get();
    let selected_jot = Select::new("Select a note to modify", set.jots.clone()).prompt()?;
    
    // equality works here because if two notes are identical then changing either one has a
    // functionally equivalent effect. 
    let (selected_index, _) = set.jots.iter().enumerate().find(|(i, ref jot)| jot == &&selected_jot).unwrap();

    let new_message = if let Some(new_message) = args.jot {
        new_message
    } else {
        Text::new("Provide new jot message").with_initial_value(&selected_jot.value).prompt()?
    };

    let new_state = if let Some(new_state) = args.state {
        new_state
    } else {
        prompt_state()?
    };

    let new_jot = Jot{value: new_message, state: new_state};
    history.set_jot(new_jot, selected_index);

    Ok(())
}

fn view_history(history: &JotHistory) -> Result<()> {
    let intervals = history.get_date_intervals();
    let selected_interval = Select::new("Select a date interval to view", intervals.clone()).prompt()?;
    let (set_index, _) = intervals.iter().enumerate().find(|(i, interval)| matches!(interval, selected_interval)).unwrap();
    println!("{}", history.sets.get(set_index).unwrap());
    Ok(())
}

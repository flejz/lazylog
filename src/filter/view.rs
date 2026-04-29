use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use crossbeam_channel::{Receiver, Sender};
use crate::parser::{FormatHint, parse_line};
use super::FilterState;
use crate::filter::LevelRegistry;

#[derive(Debug, Clone)]
pub enum FilterSource {
    File(PathBuf),
    Lines(Vec<Vec<u8>>), // snapshot for ring/stdin
}

#[derive(Debug)]
pub enum WorkerCmd {
    RecomputeFilter {
        generation: u64,
        filter: FilterState,
        format: FormatHint,
        source: FilterSource,
    },
    Cancel,
}

#[derive(Debug)]
pub enum FilterMsg {
    Chunk {
        generation: u64,
        indices: Vec<u32>,
        progress: f64,
    },
    Done {
        generation: u64,
        total_matched: u64,
    },
    Cancelled {
        generation: u64,
    },
}

pub struct FilterWorker;

impl FilterWorker {
    pub fn spawn(
        cmd_rx: Receiver<WorkerCmd>,
        result_tx: Sender<FilterMsg>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            run_worker(cmd_rx, result_tx);
        })
    }
}

fn run_worker(cmd_rx: Receiver<WorkerCmd>, result_tx: Sender<FilterMsg>) {
    for cmd in &cmd_rx {
        match cmd {
            WorkerCmd::Cancel => {}
            WorkerCmd::RecomputeFilter { generation, filter, format, source } => {
                let registry = LevelRegistry::new();
                let mut matched_indices: Vec<u32> = Vec::new();
                let mut total_matched: u64 = 0;

                match source {
                    FilterSource::File(path) => {
                        let Ok(file) = std::fs::File::open(&path) else {
                            let _ = result_tx.send(FilterMsg::Done { generation, total_matched: 0 });
                            continue;
                        };
                        let reader = BufReader::new(file);
                        let file_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(1).max(1);
                        let mut bytes_read: u64 = 0;
                        let mut line_no: u32 = 0;

                        for raw in reader.split(b'\n') {
                            let Ok(bytes) = raw else { break };
                            bytes_read += bytes.len() as u64 + 1;

                            // Check for cancellation every 10k lines
                            if line_no.is_multiple_of(10_000) {
                                if let Ok(new_cmd) = cmd_rx.try_recv() {
                                    match new_cmd {
                                        WorkerCmd::Cancel | WorkerCmd::RecomputeFilter { .. } => {
                                            let _ = result_tx.send(FilterMsg::Cancelled { generation });
                                            return; // exit the thread; new cmd needs fresh iteration
                                        }
                                    }
                                }
                            }

                            let log_line = parse_line(&bytes, line_no as u64, format);
                            let level_ok = filter.level_visible(log_line.level, &registry);
                            let crate_ok = filter.crate_visible(log_line.target.as_deref());
                            let time_ok = filter.time_visible(log_line.timestamp.as_deref());
                            if level_ok && crate_ok && time_ok {
                                matched_indices.push(line_no);
                                total_matched += 1;
                            }

                            if matched_indices.len() >= 4096 {
                                let progress = (bytes_read as f64 / file_len as f64).min(1.0);
                                let _ = result_tx.send(FilterMsg::Chunk {
                                    generation,
                                    indices: std::mem::take(&mut matched_indices),
                                    progress,
                                });
                            }

                            line_no += 1;
                        }

                        if !matched_indices.is_empty() {
                            let _ = result_tx.send(FilterMsg::Chunk {
                                generation,
                                indices: std::mem::take(&mut matched_indices),
                                progress: 1.0,
                            });
                        }
                    }

                    FilterSource::Lines(lines) => {
                        let _total = lines.len();
                        for (line_no, bytes) in lines.iter().enumerate() {
                            let log_line = parse_line(bytes, line_no as u64, format);
                            let level_ok = filter.level_visible(log_line.level, &registry);
                            let crate_ok = filter.crate_visible(log_line.target.as_deref());
                            let time_ok = filter.time_visible(log_line.timestamp.as_deref());
                            if level_ok && crate_ok && time_ok {
                                matched_indices.push(line_no as u32);
                                total_matched += 1;
                            }
                        }
                        let _ = result_tx.send(FilterMsg::Chunk {
                            generation,
                            indices: matched_indices,
                            progress: 1.0,
                        });
                    }
                }

                let _ = result_tx.send(FilterMsg::Done { generation, total_matched });
            }
        }
    }
}

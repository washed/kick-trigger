use std::env;
use std::io::Bytes;
use std::path::Path;

use anyhow::anyhow;
use crossterm::event;
use defmt_decoder::{DecodeError, Frame, Locations, Table};
use probe_rs::probe::list::Lister;
use probe_rs::rtt::Rtt;
use probe_rs::Permissions;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Chart, Dataset, GraphType, StatefulWidget, Widget};
use ratatui::Frame as RFrame;
use std::sync::{Arc, RwLock};
use tokio::fs;
use tokio::signal::ctrl_c;

const READ_BUFFER_SIZE: usize = 1024;

#[derive(Copy, Clone)]
struct GraphWidget {}

impl StatefulWidget for GraphWidget {
    type State = Arc<RwLock<Vec<u16>>>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Arc<RwLock<Vec<u16>>>) {
        let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).spacing(1);
        let [top, main] = area.layout(&layout);

        let title = Line::from_iter([
            Span::from("Chart Widget").bold(),
            Span::from(" (Press 'q' to quit)"),
        ]);

        title.centered().render(top, buf);

        let state_guard = state.read().unwrap();

        let data: Vec<(f64, f64)> = state_guard
            .iter()
            .enumerate()
            .map(|(i, v)| (i as f64, *v as f64))
            .collect();

        drop(state_guard);

        let dataset_upward = Dataset::default()
            .name("Stonks")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Blue)
            .data(data.as_slice());

        let x_axis = Axis::default()
            .title("Hustle".blue())
            .bounds([0.0, 2000.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Profit".blue())
            .bounds([0.0, 4096.0])
            .labels(["0", "10", "20"]);

        let chart = Chart::new(vec![dataset_upward])
            .x_axis(x_axis)
            .y_axis(y_axis);
        chart.render(main, buf);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    color_eyre::install().unwrap();

    let state = Arc::new(RwLock::new(Vec::new()));
    let mut state_read = state.clone();
    let graph = GraphWidget {};
    tokio::spawn(async move {
        ratatui::run(|terminal| loop {
            terminal
                .draw(|frame| frame.render_stateful_widget(graph, frame.area(), &mut state_read))
                .unwrap();
            if event::read().unwrap().is_key_press() {
                break;
            }
        });
    });

    let bytes = fs::read("target/thumbv7em-none-eabihf/release/kick-trigger").await?;
    let table = Table::parse(&bytes)?.ok_or_else(|| anyhow!(".defmt data not found"))?;
    let locs = table.get_locations(&bytes)?;

    // check if the locations info contains all the indicies
    let locs = if table.indices().all(|idx| locs.contains_key(&(idx as u64))) {
        Some(locs)
    } else {
        eprintln!("(BUG) location info is incomplete; it will be omitted from the output");
        None
    };

    let mut stream_decoder = table.new_stream_decoder();
    let current_dir = env::current_dir()?;

    // First obtain a probe-rs session (see probe-rs documentation for details)
    let lister = Lister::new();

    let probes = lister.list_all();
    // println!("{:?}", probes);

    let probe = probes[0].open().unwrap();

    let mut session = probe
        .attach("STM32L476RGTx", Permissions::default())
        .unwrap();
    // Select a core.
    let mut core = session.core(0).unwrap();

    // Attach to RTT
    let mut rtt = Rtt::attach(&mut core).unwrap();

    let mut buf = [0; READ_BUFFER_SIZE];

    let cancel_task = tokio::spawn(async move { ctrl_c().await });

    let state_write = state.clone();
    // Read from a channel
    loop {
        if let Some(input) = rtt.up_channel(0) {
            let count = input.read(&mut core, &mut buf[..]).unwrap();

            if count == 0 {
                continue;
            } else {
                // println!("Read data: {:?}", &buf[..count]);
            }

            stream_decoder.received(&buf[..count]);

            loop {
                if cancel_task.is_finished() {
                    break;
                }

                match stream_decoder.decode() {
                    Ok(frame) => {
                        forward_to_logger(&frame, location_info(&locs, &frame, &current_dir));

                        let msg = format_args!("{}", frame.display_message());
                        let str = format!("{:?}", msg);

                        if str.starts_with("[") && str.ends_with("]") {
                            let byte_strs = &str[1..str.len() - 1].split(",");
                            let bytes: Vec<u8> = byte_strs
                                .clone()
                                .map(|s| {
                                    let byte: u8 = s.trim().parse().unwrap();
                                    byte
                                })
                                .collect();

                            // println!("{:?}", bytes);

                            let pc: Vec<u16> = postcard::from_bytes(bytes.as_slice()).unwrap();

                            let mut state_guard = state_write.write().unwrap();
                            *state_guard = pc;
                            drop(state_guard);

                            // println!("{:?}", pc);
                        }
                    }
                    Err(DecodeError::UnexpectedEof) => break,
                    Err(DecodeError::Malformed) => match table.encoding().can_recover() {
                        // if recovery is impossible, abort
                        false => return Err(DecodeError::Malformed.into()),
                        // if recovery is possible, skip the current frame and continue with new data
                        true => {
                            // bug: https://github.com/rust-lang/rust-clippy/issues/9810
                            // println!("(HOST) malformed frame skipped");
                            // println!("└─ {} @ {}:{}", env!("CARGO_PKG_NAME"), file!(), line!());
                            continue;
                        }
                    },
                }
            }
        }
        if cancel_task.is_finished() {
            break;
        }
    }

    // probe.detach().unwrap();

    Ok(())
}

type LocationInfo = (Option<String>, Option<u32>, Option<String>);

fn forward_to_logger(frame: &Frame, location_info: LocationInfo) {
    let (file, line, mod_path) = location_info;

    let msg = format_args!("{}", frame.display_message());
    let str = format!("{:?}", msg);

    if str.starts_with("[") && str.ends_with("]") {
    } else {
        // println!("{}", str);
    }
}

fn location_info(locs: &Option<Locations>, frame: &Frame, current_dir: &Path) -> LocationInfo {
    let (mut file, mut line, mut mod_path) = (None, None, None);

    let loc = locs.as_ref().map(|locs| locs.get(&frame.index()));

    if let Some(Some(loc)) = loc {
        // try to get the relative path, else the full one
        let path = loc.file.strip_prefix(current_dir).unwrap_or(&loc.file);

        file = Some(path.display().to_string());
        line = Some(loc.line as u32);
        mod_path = Some(loc.module.clone());
    }

    (file, line, mod_path)
}

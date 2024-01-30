
// new Proactive IO
use nuclei::*;
#[allow(unused_imports)]
use futures::io::SeekFrom;
#[allow(unused_imports)]
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use std::fs::{create_dir_all, File, OpenOptions};

use std::path::PathBuf;

use std::sync::Arc;
use bastion::run;

use std::fmt::Write;
use std::vec;
use bytes::{BufMut, BytesMut};
use log::*;
use time::{format_description, OffsetDateTime};
use uuid::Uuid;
use crate::steady_state::{ChannelDataType, ColorTrigger};
use crate::steady_state::{config};
use crate::steady_state::monitor::ChannelMetaData;
use crate::steady_state::serialize::byte_buffer_packer::PackedVecWriter;
use crate::steady_state::serialize::fast_protocol_packed::write_long_unsigned;

pub struct DotState {
    pub(crate) nodes: Vec<Node<>>, //position matches the node id
    pub(crate) edges: Vec<Edge<>>, //position matches the channel id
    pub seq: u64,
}

pub(crate) struct Node {
    pub id: usize,
    pub color: & 'static str,
    pub pen_width: & 'static str,
    pub display_label: String, //label may also have (n) for replicas
}

pub(crate) struct Edge {
    pub(crate) id: usize, //position matches the channel id
    pub(crate) from: usize, //monitor actor id of the sender
    pub(crate) to: usize, //monitor actor id of the receiver
    pub(crate) color: & 'static str, //results from computer
    pub(crate) pen_width: & 'static str, //results from computer
    pub(crate) display_label: String, //results from computer
    pub(crate) ctl_labels: Vec<&'static str>, //visibility tags for render
    stats_computer: ChannelStatsComputer
}
impl Edge {
    pub(crate) fn compute_and_refresh(&mut self, send: i128, take: i128) {
        let (label,color,pen) = self.stats_computer.compute(send, take);
        self.display_label = label;
        self.color = color;
        self.pen_width = pen;
    }
}


pub fn build_dot(state: &DotState, rankdir: &str, dot_graph: &mut BytesMut) {
    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");
    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    for node in &state.nodes {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(node.id.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    for edge in &state.edges {

        // TODO: new feature must also drop nodes if no edges are visible
       // let show = edge.ctl_labels.iter().any(|f| config::is_visible(f));


        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(edge.from.to_string().as_bytes());
        dot_graph.put_slice(b"\" -> \"");
        dot_graph.put_slice(edge.to.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(edge.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(edge.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(edge.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    dot_graph.put_slice(b"}\n");
}



pub struct DotGraphFrames {
    pub(crate) active_graph: BytesMut,

}


const DOT_RED: & 'static str = "red";
const DOT_GREEN: & 'static str = "green";
const DOT_YELLOW: & 'static str = "yellow";
const DOT_WHITE: & 'static str = "white";
const DOT_GREY: & 'static str = "grey";
const DOT_BLACK: & 'static str = "black";

static DOT_PEN_WIDTH: [&'static str; 16]
= ["1", "2", "3", "5", "8", "13", "21", "34", "55", "89", "144", "233", "377", "610", "987", "1597"];

const SQUARE_LIMIT: u128 = (1 << 64)-1; // (u128::MAX as f64).sqrt() as u128;


struct ChannelStatsComputer {
    pub(crate) display_labels: Option<Vec<& 'static str>>,
    pub(crate) line_expansion: bool,
    pub(crate) show_type: Option<& 'static str>,
    pub(crate) percentiles: Vec<ChannelDataType>, //each is a row
    pub(crate) std_devs: Vec<ChannelDataType>, //each is a row
    pub(crate) red_trigger: Vec<ColorTrigger>, //if used base is green
    pub(crate) yellow_trigger: Vec<ColorTrigger>, //if used base is green

    pub(crate) runner_inflight:u128,
    pub(crate) runner_consumed:u128,
    pub(crate) percentile_inflight:[u32;65],//based on u64.leading_zeros() we wil put the answers in that bucket
    pub(crate) percentile_consumed:[u32;65],//based on u64.leading_zeros() we wil put the answers in that bucket
    pub(crate) buckets_inflight:Vec<u64>,
    pub(crate) buckets_consumed:Vec<u64>,
    pub(crate) buckets_index:usize,
    pub(crate) buckets_mask:usize,
    pub(crate) buckets_bits:u8,

    pub(crate) time_label: String,
    pub(crate) prev_take: i128,
    pub(crate) has_full_window: bool,
    pub(crate) capacity: usize,

    pub(crate) sum_of_squares_inflight:u128,
    pub(crate) sum_of_squares_consumed:u128,

    pub(crate) show_avg_inflight:bool,
    pub(crate) show_avg_consumed:bool,
}
impl ChannelStatsComputer {

    pub(crate) fn new(meta: &Arc<ChannelMetaData>) -> ChannelStatsComputer {

        let display_labels = if meta.display_labels {
            Some(meta.labels.clone())
        } else {
            None
        };
        let line_expansion = meta.line_expansion;
        let show_type = meta.show_type;
        let percentiles                       = meta.percentiles.clone();
        let std_devs                          = meta.std_dev.clone();
        let red_trigger                       = meta.red.clone();
        let yellow_trigger                    = meta.yellow.clone();

        let buckets_bits = meta.window_bucket_in_bits;
        let buckets_count:usize = 1<<meta.window_bucket_in_bits;
        let buckets_mask:usize = buckets_count - 1;
        let buckets_index:usize = 0;
        let time_label = time_label(buckets_count * config::TELEMETRY_PRODUCTION_RATE_MS);

        let buckets_inflight:Vec<u64> = vec![0u64; buckets_count];
        let buckets_consumed:Vec<u64> = vec![0u64; buckets_count];
        let runner_inflight:u128      = 0;
        let runner_consumed:u128      = 0;
        let percentile_inflight       = [0u32;65];
        let percentile_consumed       = [0u32;65];

        let  prev_take = 0i128;

        let sum_of_squares_inflight:u128= 0;
        let sum_of_squares_consumed:u128= 0;
        let capacity = meta.capacity;
        let show_avg_inflight = meta.avg_inflight;
        let show_avg_consumed = meta.avg_consumed;



        ChannelStatsComputer {
            display_labels,
            line_expansion,
            show_type,
            percentiles,
            std_devs,
            red_trigger,
            yellow_trigger,
            runner_inflight,
            runner_consumed,
            percentile_inflight,
            percentile_consumed,
            buckets_inflight,
            buckets_consumed,
            buckets_mask,
            buckets_bits,
            buckets_index,
            time_label,
            prev_take,
            sum_of_squares_inflight,
            sum_of_squares_consumed,
            has_full_window : false,
            capacity,
            show_avg_inflight,
            show_avg_consumed,

        }
    }

    pub(crate) fn compute(&mut self, send: i128, take: i128)
        -> (String, & 'static str, & 'static str) {

        assert!(send>=take, "internal error send {} must be greater or eq than take {}",send,take);

        // compute the running totals

        let inflight:u64 = (send-take) as u64;
        let consumed:u64 = (take-self.prev_take) as u64;

        self.runner_inflight += inflight as u128;
        self.runner_inflight -= self.buckets_inflight[self.buckets_index] as u128;

        self.percentile_inflight[64-inflight.leading_zeros() as usize] += 1u32;
        let old_zeros = self.buckets_inflight[self.buckets_index].leading_zeros() as usize;
        self.percentile_inflight[old_zeros] = self.percentile_inflight[64-old_zeros].saturating_sub(1u32);

        let inflight_square = (inflight as u128).pow(2);
        self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_add(inflight_square);
        self.sum_of_squares_inflight = self.sum_of_squares_inflight.saturating_sub((self.buckets_inflight[self.buckets_index] as u128).pow(2));
        self.buckets_inflight[self.buckets_index] = inflight as u64;

        self.runner_consumed += consumed as u128;
        self.runner_consumed -= self.buckets_consumed[self.buckets_index] as u128;

        self.percentile_consumed[64-consumed.leading_zeros() as usize] += 1;
        let old_zeros = self.buckets_consumed[self.buckets_index].leading_zeros() as usize;
        self.percentile_consumed[old_zeros] = self.percentile_consumed[64-old_zeros].saturating_sub(1);

        let consumed_square = (consumed as u128).pow(2);
        self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_add(consumed_square);
        self.sum_of_squares_consumed = self.sum_of_squares_consumed.saturating_sub((self.buckets_consumed[self.buckets_index] as u128).pow(2));
        self.buckets_consumed[self.buckets_index] = consumed as u64;

        self.buckets_index = (1+self.buckets_index) & self.buckets_mask;
        self.prev_take = take;

        //  build the label

        let mut display_label = String::new();

        self.show_type.map(|f| {
            display_label.push_str(f);
            display_label.push_str("\n");
        });

        display_label.push_str("per ");
        display_label.push_str(&self.time_label);
        display_label.push_str("\n");

        self.display_labels.as_ref().map(|labels| {
            //display_label.push_str("labels: ");
            labels.iter().for_each(|f| {
                display_label.push_str(f);
                display_label.push_str(" ");
            });
            display_label.push_str("\n");
        });

        let window = 1usize<<self.buckets_bits;

        if self.show_avg_consumed {
            let avg = self.runner_consumed as f64 / window as f64;
            display_label.push_str(format!("consumed avg: {:.1} ", avg).as_str());
            display_label.push_str("\n");
        }
        if self.show_avg_inflight {
            let avg = self.runner_inflight as f64 / window as f64;
            display_label.push_str(format!("inflight avg: {:.1} ", avg).as_str());
            display_label.push_str("\n");
        }

        //do not compute the std_devs if we do not have a full window of data.
        if self.has_full_window {
            self.std_devs.iter().for_each(|f| {
               let (label,mult,runner,sum_sqr) = match f {
                    ChannelDataType::InFlight(mult) => {
                        //display_label.push_str("InFlight ");
                        let runner = self.runner_inflight;
                        let sum_sqr = self.sum_of_squares_inflight;
                        ("inflight",mult,runner,sum_sqr)
                    }
                    ChannelDataType::Consumed(mult) => {
                        //display_label.push_str("Consumed ");
                        let runner = self.runner_consumed;
                        let sum_sqr = self.sum_of_squares_consumed;
                        ("consumed",mult,runner,sum_sqr)
                    }
                };
                let std_deviation = Self::compute_variance(self.buckets_bits, window, runner, sum_sqr).sqrt();
                display_label.push_str(format!("{} {:.1}StdDev: {:.1} "
                                               , label,mult, mult*std_deviation as f32
                                               ).as_str());
                display_label.push_str("\n");
            });
        }




        if self.has_full_window && self.percentiles.len() > 0 {
           // display_label.push_str("Percentiles\n");
            self.percentiles.iter().for_each(|p| {

                let (pct, label, walker) = match p {
                    ChannelDataType::InFlight(pct) => {
                        let walker = Self::compute_percentile(window as u32, *pct, self.percentile_inflight);
                        (pct, "inflight", walker)
                    }
                    ChannelDataType::Consumed(pct) => {
                        let walker = Self::compute_percentile(window as u32, *pct, self.percentile_inflight);
                        (pct, "consumed", walker)
                    }
                };

                display_label.push_str(format!("{} {:?}%ile {:?}"
                                               , label, pct*100f32
                                               , 1u64<<(walker+1) ).as_str());

            });
            display_label.push_str("\n");
        }

        display_label.push_str(format!("Capacity: {} Total: {}\n",self.capacity,take).as_str());

        let line_thick = if self.line_expansion {
            DOT_PEN_WIDTH[ (128usize-(take>>20).leading_zeros() as usize).min(DOT_PEN_WIDTH.len()-1) ]
        } else {
            "1"
        };

        let line_color = "white";
        /*
        let line_color = if let Some(red) = &self.red_trigger {
            if red.trigger(take) {
                DOT_RED
            } else {
                line_color
            }
        } else {
            line_color
        };

        */


        self.has_full_window = self.has_full_window || self.buckets_index == 0;


        (display_label,line_color,line_thick)

    }

    fn compute_percentile(window: u32, pct: f32, ptable: [u32; 65]) -> usize {
        let limit: u32 = (window as f32 * pct) as u32;
        let mut walker = 0usize;
        let mut sum = 0u32;
        while walker < 62 && sum + ptable[walker] < limit {
            sum += ptable[walker];
            walker += 1;
        }
        walker
    }

    #[inline]
    fn compute_variance(bits: u8, window: usize, runner: u128, sum_sqr: u128) -> f64 {
            if runner < SQUARE_LIMIT {
                let r2 = (runner*runner)>>bits;
                //trace!("{} {} {} {} -> {} ",bits,window,runner,sum_sqr,r2);
                if sum_sqr > r2 {
                    ((sum_sqr - r2) >> bits) as f64
                } else {
                    (sum_sqr as f64 / window as f64) - (runner as f64 / window as f64).powi(2)
                }
            } else {
                //trace!("{:?} {:?} {:?} {:?} " ,bits ,window,runner ,sum_sqr);

                (sum_sqr as f64 / window as f64) - (runner as f64 / window as f64).powi(2)
            }
    }
}

fn time_label(total_ms: usize) -> String {
    let seconds = total_ms as f64 / 1000.0;
    let minutes = seconds / 60.0;
    let hours = minutes / 60.0;
    let days = hours / 24.0;

    if days >= 1.0 {
        if days< 1.1 {"day".to_string()} else {
            format!("{:.1} days", days)
        }
    } else if hours >= 1.0 {
        if hours< 1.1 {"hr".to_string()} else {
            format!("{:.1} hrs", hours)
        }
    } else if minutes >= 1.0 {
        if minutes< 1.1 {"min".to_string()} else {
            format!("{:.1} mins", minutes)
        }
    } else {
        if seconds< 1.1 {"sec".to_string()} else {
            format!("{:.1} secs", seconds)
        }
    }
}



pub fn refresh_structure(mut local_state: &mut DotState
                         , name: &str
                         , id: usize
                         , channels_in: Arc<Vec<Arc<ChannelMetaData>>>
                         , channels_out: Arc<Vec<Arc<ChannelMetaData>>>
) {
//rare but needed to ensure vector length
    if id.ge(&local_state.nodes.len()) {
        local_state.nodes.resize_with(id + 1, || {
            Node {
                id: usize::MAX,
                color: "grey",
                pen_width: "2",
                display_label: "".to_string(),
            }
        });
    }
    local_state.nodes[id].id = id;
    local_state.nodes[id].display_label = name.to_string();

    //edges are defined by both the sender and the receiver
    //we need to record both monitors in this edge as to and from
    define_unified_edges(&mut local_state, id, channels_in, true);
    define_unified_edges(&mut local_state, id, channels_out, false);


}

fn define_unified_edges(mut local_state: &mut &mut DotState, node_id: usize, mdvec: Arc<Vec<Arc<ChannelMetaData>>>, set_to: bool) {
    mdvec.iter()
        .for_each(|meta| {
            if meta.id.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(meta.id + 1, || {
                    Edge {
                        id: usize::MAX,
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        stats_computer: ChannelStatsComputer::new(meta),
                        ctl_labels: Vec::new(), //for visibility control
                    }
                });
            }
            let idx = meta.id;
            assert!(local_state.edges[idx].id == idx || local_state.edges[idx].id == usize::MAX);
            local_state.edges[idx].id = idx;
            if set_to {
                local_state.edges[idx].to = node_id;
            } else {
                local_state.edges[idx].from = node_id;
            }
            // Collect the labels that need to be added
            // This is redundant but provides safety if two dif label lists are in play
            let labels_to_add: Vec<_> = meta.labels.iter()
                .filter(|f| !local_state.edges[idx].ctl_labels.contains(f))
                .cloned()  // Clone the items to be added
                .collect();
            // Now, append them to `ctl_labels`
            for label in labels_to_add {
                local_state.edges[idx].ctl_labels.push(label);
            }
        });
}

//////////////////////////
/////////////////////////

pub(crate) struct FrameHistory {
    pub(crate) packed_sent_writer: PackedVecWriter<i128>,
    pub(crate) packed_take_writer: PackedVecWriter<i128>,
    pub(crate) history_buffer: BytesMut,
    pub(crate) guid: String,
    pub(crate) format: Vec<format_description::FormatItem<'static>>,
    output_log_path: PathBuf,
    file_bytes_written: usize,
    file_buffer_last_seq: u64,
    last_file_to_append_onto: String,
    buffer_bytes_count: usize,
}

const REC_NODE:u64 = 1;
const REC_EDGE:u64 = 0;

const HISTORY_WRITE_BLOCK_SIZE:usize = 1 << 13; //13 is 8K NOTE: must be power of 2 and 4096 or larger

impl FrameHistory {

    pub async fn new() -> FrameHistory {
        let result = FrameHistory {
            packed_sent_writer: PackedVecWriter { //TODO: use a constructor
                previous: Vec::new(),
                sync_required: true, //first must be full
                write_count: 0
            },
            packed_take_writer: PackedVecWriter {//TODO: use a constructor
                previous: Vec::new(),
                sync_required: true, //first must be full
                write_count: 0
            },
            history_buffer: BytesMut::new(),
            //immutable details
            guid: Uuid::new_v4().to_string(), // Unique GUID for the run instance
            format: format_description::parse("[year]_[month]_[day]").expect("Invalid format description"),
            output_log_path: PathBuf::from("output_logs"),
            //running state
            file_bytes_written: 0usize,
            file_buffer_last_seq: 0u64,
            last_file_to_append_onto: "".to_string(),
            buffer_bytes_count: 0usize,
        };
        run!(async {let _ = create_dir_all(&result.output_log_path);});
        result
    }

    pub fn mark_position(&mut self) {
        self.buffer_bytes_count = self.history_buffer.len();
    }

    pub fn apply_node(&mut self, name: & 'static str, id:usize
                      , chin: Arc<Vec<Arc<ChannelMetaData>>>
                      , chout: Arc<Vec<Arc<ChannelMetaData>>>) {

        write_long_unsigned(REC_NODE, &mut self.history_buffer); //message type
        write_long_unsigned(id as u64, &mut self.history_buffer); //message type

        write_long_unsigned(name.len() as u64,&mut self.history_buffer);
        self.history_buffer.write_str(name).expect("internal error writing to ByteMut");

            write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
            chin.iter().for_each(|meta|{
                write_long_unsigned(meta.id as u64, &mut self.history_buffer);
                write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
                meta.labels.iter().for_each(|s| {
                    write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                    self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                });
            });

            write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
            chout.iter().for_each(|meta|{
                write_long_unsigned(meta.id as u64, &mut self.history_buffer);
                write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
                meta.labels.iter().for_each(|s| {
                    write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                    self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                });
            });
    }


    pub fn apply_edge(&mut self, total_take:Vec<i128>, total_send: Vec<i128>) {
             write_long_unsigned(REC_EDGE, &mut self.history_buffer); //message type
            self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);
            self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);

    }


    pub async fn update(&mut self, sequence: u64, flush_all: bool) {
        //we write to disk in blocks just under a fixed power of two size
        //if we are about to enter a new block ensure we write the old one
        if flush_all || self.will_span_into_next_block()  {
            let mut continued_buffer:BytesMut = self.history_buffer.split_off(self.buffer_bytes_count);
            let log_time = OffsetDateTime::now_utc();
            let file_to_append_onto = format!("{}_{}_log.dat"
                                              , log_time
                                                  .format(&self.format)
                                                  .unwrap_or("0000_00_00".to_string())
                                              , self.guid);

            //if we are starting a new file reset our counter to zero
            if !self.last_file_to_append_onto.eq(&file_to_append_onto) {
                self.file_bytes_written = 0;
                self.last_file_to_append_onto=file_to_append_onto.clone();
            }
            info!("attempt to write history");
            let to_be_written = std::mem::replace(&mut self.history_buffer, continued_buffer);
            if let Err(e) = Self::append_to_file(self.output_log_path.join(&file_to_append_onto)
                                                , to_be_written).await {
                error!("Error writing to file: {}", e);
                error!("Due to the above error some history has been lost");

                //we force a full write for the next time around
                self.packed_take_writer.sync_data();
                self.packed_sent_writer.sync_data();

                //NOTE: we could hold for write later but we may run out of memory
                //      so we will just drop the data and hope for the best

            } else {
                info!("wrote to hist log now {} bytes",self.file_bytes_written+self.buffer_bytes_count);
            }
            self.file_bytes_written += self.buffer_bytes_count;

        }
        //set after in both cases right now the data matches this sequence
        self.file_buffer_last_seq = sequence;
    }

    fn will_span_into_next_block(&self) -> bool {
            let old_blocks = (self.file_bytes_written + self.buffer_bytes_count) / HISTORY_WRITE_BLOCK_SIZE;
            let new_blocks = (self.file_bytes_written + self.history_buffer.len()) / HISTORY_WRITE_BLOCK_SIZE;
            new_blocks > old_blocks
    }


    async fn append_to_file(path: PathBuf, data: BytesMut) -> Result<(), std::io::Error> {

                    let file = OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(&path)?;

                    let h = Handle::<File>::new(file)?;
                    nuclei::spawn(Self::all_to_file_async(h, data)).await

    }
    pub(crate) async fn all_to_file_async(mut h: Handle<File>, data: BytesMut) -> Result<(), std::io::Error> {
        h.write_all(data.as_ref()).await?;
        Handle::<File>::flush(&mut h).await?;
        Ok(())
    }

}


use clap::Parser;
use disarm64_defn::deser::Insn;
use disarm64_defn::InsnClass;
use disarm64_defn::InsnFeatureSet;
use disarm64_defn::InsnFlags;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use crate::generate_graphviz_dot::decistion_tree_to_graphviz_dot;
use crate::generate_rust::decision_tree_to_rust;
use decision_tree::build_decision_tree;
use decision_tree::DecisionTreeIndexing;

mod decision_tree;
mod generate_graphviz_dot;
mod generate_rust;
mod generate_test_bin;

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
enum DecoderAlgo {
    /// Conditionals
    Cond,
    /// DFS table-driven
    Dfs,
    /// BFS table-driven
    Bfs,
}

#[derive(Parser, Debug)]
/// This tool generates an instruction decoder from a JSON description of the ISA.
struct CommandLine {
    /// A JSON file with the description of the instruction set architecture.
    description_json: PathBuf,
    /// Decoder algorithm style, defaults to conditionals.
    #[clap(short = 'a', long)]
    algo: Option<DecoderAlgo>,
    /// Include filter for feature sets, e.g. "v8,simd".
    /// Case-insensitive, ignored if not provided.
    #[clap(short = 'f', long, value_delimiter = ',', num_args = 1..)]
    feature_sets: Option<Vec<InsnFeatureSet>>,
    /// Include filter for instruction classes, e.g. "addsub_imm,ldst_pos,exception".
    /// Case-insensitive, ignored if not provided.
    #[clap(short = 'c', long, value_delimiter = ',', num_args = 1..)]
    insn_class: Option<Vec<InsnClass>>,
    /// Include filter for mnemonics, e.g. "adc,ldp".
    /// Case-insensitive, ignored if not provided.
    #[clap(short = 'm', long, value_delimiter = ',', num_args = 1..)]
    mnemonic: Option<Vec<String>>,
    /// Output the decision tree to a Graphviz DOT file.
    #[clap(short, long)]
    graphviz: Option<PathBuf>,
    /// Generate the decoder implemented in Rust.
    #[clap(short, long)]
    rs_file: Option<PathBuf>,
    /// Generate a test binary.
    #[clap(short, long)]
    test_bin: Option<PathBuf>,
    /// The size limit of the generated test binary, the default is 64MB.
    #[clap(long, default_value = "67108864")]
    test_bin_size_limit: usize,
    /// The number of test encodings to generate for each instruction, the default is 0x10_000.
    #[clap(long, default_value = "65536")]
    test_encodings_limit: usize,
    /// Log level/verbosity; repeat (-v, -vv, ...) to increase the verbosity.
    #[clap(short, action = clap::ArgAction::Count)]
    verbosity: u8,
}

fn init_logging(opt: &CommandLine) {
    // Maybe:
    // /// Log level.
    // #[clap(long, default_value = "info")]
    // log: log::LevelFilter,

    // From the env variable:
    // env_logger::Builder::from_env().init();

    env_logger::builder()
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false)
        .filter(
            None,
            match opt.verbosity {
                0 => log::LevelFilter::Info,
                1 => log::LevelFilter::Debug,
                _ => log::LevelFilter::Trace,
            },
        )
        .init();
}

fn main() -> anyhow::Result<()> {
    let opt = CommandLine::parse();

    init_logging(&opt);

    let filter_feature_sets = HashSet::from_iter(opt.feature_sets.unwrap_or_default());
    let filter_insn_class = HashSet::from_iter(opt.insn_class.unwrap_or_default());
    let filter_mnemonic = HashSet::from_iter(opt.mnemonic.unwrap_or_default());
    let insns = parse_insn_description(
        opt.description_json,
        filter_feature_sets,
        filter_insn_class,
        filter_mnemonic,
    )?;

    let decision_tree_indexing = match opt.algo {
        Some(DecoderAlgo::Cond) | None => DecisionTreeIndexing::None,
        Some(DecoderAlgo::Bfs) => DecisionTreeIndexing::BFS,
        Some(DecoderAlgo::Dfs) => DecisionTreeIndexing::DFS,
    };
    let decision_tree = build_decision_tree(insns.as_slice(), decision_tree_indexing);
    if let Some(graphviz) = opt.graphviz {
        log::info!("Writing decision tree to a Graphviz dot file {graphviz:?}");
        let mut f = std::fs::File::create(graphviz)?;
        decistion_tree_to_graphviz_dot(&decision_tree, &mut f)?;
    }

    if let Some(rust) = opt.rs_file {
        log::info!("Writing decision tree to a Rust file {rust:?}");
        let mut f = std::fs::File::create(rust)?;
        decision_tree_to_rust(&decision_tree, decision_tree_indexing, &mut f)?;
    }

    if let Some(test_bin) = opt.test_bin {
        log::info!(
            "Generating test binary {test_bin:?}, limit {} bytes, {} encodings per instruction",
            opt.test_bin_size_limit,
            opt.test_encodings_limit
        );
        let mut f = std::fs::File::create(test_bin)?;
        generate_test_bin::generate_test_bin(
            insns.as_slice(),
            &mut f,
            opt.test_bin_size_limit,
            opt.test_encodings_limit,
        )?;
    }

    Ok(())
}

fn parse_insn_description(
    path: PathBuf,
    feature_sets_filter: HashSet<InsnFeatureSet>,
    insn_class_filter: HashSet<InsnClass>,
    mnemonic_filter: HashSet<String>,
) -> anyhow::Result<Vec<Rc<Insn>>> {
    log::info!("Loading {path:?}");

    if !feature_sets_filter.is_empty() {
        log::info!("Including instructions from feature sets {feature_sets_filter:?}");
    } else {
        log::info!("Including instructions from all feature sets");
    }
    if !insn_class_filter.is_empty() {
        log::info!("Including instructions from classes {insn_class_filter:?}");
    } else {
        log::info!("Including instructions from all classes");
    }
    if !mnemonic_filter.is_empty() {
        log::info!("Including instructions with mnemonics {mnemonic_filter:?}");
    } else {
        log::info!("Including instructions with all mnemonics");
    }

    let data = std::fs::read_to_string(path)?;
    let insns = serde_json::from_str::<Vec<Insn>>(&data)?;
    let mut filtered_insns = Vec::new();

    let mut aliases = 0;
    let mut insn_classes = HashSet::new();
    let mut insn_feature_sets = HashSet::new();
    let mut i = 0;

    loop {
        if i >= insns.len() {
            break;
        }

        let insn = &insns[i];
        let opcode = insn.opcode;
        let mask = insn.mask;

        if insn.flags.contains(InsnFlags::IS_ALIAS) {
            log::trace!("skipping alias {insn:x?}");

            aliases += 1;
        } else if (feature_sets_filter.is_empty()
            || feature_sets_filter.contains(&insn.feature_set))
            && (insn_class_filter.is_empty() || insn_class_filter.contains(&insn.class))
            && (mnemonic_filter.is_empty() || mnemonic_filter.contains(&insn.mnemonic))
        {
            log::debug!("instruction {insn:x?}");

            // If opcode == opcode & mask, then additional_dont_care == 0 and opcode & !mask == 0
            let additional_dont_care = opcode ^ (mask & opcode); // == opcode & !mask
            if additional_dont_care != 0 {
                anyhow::bail!("Invalid mask for {insn:x?}, opcode & mask != mask, extra bits: {additional_dont_care:#b}");
            }

            if mask == 0 {
                anyhow::bail!("Empty mask for {insn:x?}");
            }

            insn_classes.insert(&insn.class);
            insn_feature_sets.insert(&insn.feature_set);

            filtered_insns.push(Rc::new(insn.clone()));
        } else {
            log::trace!("Skipping {insn:x?}");
        }

        i += 1;
    }

    log::debug!("Classes {insn_classes:?}");
    log::debug!("Feature sets {insn_feature_sets:?}");

    log::info!(
        "Processed {} instructions, skipped {aliases} aliases, {} classes, {} feature sets filtered out {} instructions",
        insns.len(),
        insn_classes.len(),
        insn_feature_sets.len(),
        insns.len() - filtered_insns.len()
    );

    log::info!("Loaded {} instructions", filtered_insns.len());

    Ok(filtered_insns)
}

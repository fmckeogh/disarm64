#![allow(clippy::upper_case_acronyms)]

use disarm64_defn::deser::Insn;
use std::ops::Shl;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct LeafNode {
    pub mask: u32,
    pub insn: Rc<Insn>,
}

#[derive(Debug, Clone)]
pub enum DecisionTreeNode {
    Leaf {
        index: Option<usize>,
        insns: Vec<LeafNode>,
    },
    Branch {
        index: Option<usize>,
        decision_bit: u32,
        zero: DecisionTree,
        one: DecisionTree,
    },
}

pub type DecisionTree = Option<Box<DecisionTreeNode>>;

fn build_decision_tree_recursive(
    decision_tree: &mut DecisionTree,
    insns: &[LeafNode],
    depth: &mut usize,
) {
    *depth += 1;

    log::debug!("Building decision tree at depth {}", depth);
    log::trace!("{} instructions", insns.len());

    if insns.is_empty() {
        *depth -= 1;
        log::debug!("No instructions at depth {}", depth);
        return;
    }

    if insns.len() == 1 {
        *decision_tree = Some(Box::new(DecisionTreeNode::Leaf {
            insns: insns.to_vec(),
            index: None,
        }));
        *depth -= 1;
        log::debug!("One instruction at depth {}", depth);
        return;
    }

    let mut insns = Vec::from_iter(insns.iter().cloned());
    loop {
        // Find the common bits in the mask for all instructions.
        let acc_mask = insns
            .as_slice()
            .iter()
            .fold(!0u32, |acc, insn| acc & insn.mask);
        log::debug!("mask: {:x}", acc_mask);

        // No common bits, will match one instruction at a time.
        if acc_mask == 0 {
            // Match first against the mask with the most ones.
            insns.sort_by_key(|insn| insn.mask.count_zeros());
            *decision_tree = Some(Box::new(DecisionTreeNode::Leaf {
                insns: insns.to_vec(),
                index: None,
            }));
            break;
        }

        // Find the rightmost bit that is not zero in the mask.
        let decision_bit = acc_mask.trailing_zeros();
        let decision_mask = 1u32.shl(decision_bit);
        log::debug!("decision bit: {}", decision_bit);
        log::debug!("decision mask: {:x}", decision_mask);

        // Split the instructions into two groups based on the decision bit.
        let mut zero = Vec::new();
        let mut one = Vec::new();
        for node in insns.as_slice() {
            let mut node = node.clone();
            // Clear the decision bit.
            node.mask &= !decision_mask;
            if node.insn.opcode & decision_mask == 0 {
                zero.push(node);
            } else {
                one.push(node);
            }
        }
        log::debug!("zero: {}, one: {}", zero.len(), one.len());

        // If one of the groups is empty, all instructions have the decision bit set
        // or cleared. The loop above removed it from the mask, repeat the attempt to
        // split at the next bit.
        if zero.is_empty() {
            insns = one;
            continue;
        } else if one.is_empty() {
            insns = zero;
            continue;
        }

        let mut zero_tree = None;
        let mut one_tree = None;
        build_decision_tree_recursive(&mut zero_tree, zero.as_mut_slice(), depth);
        build_decision_tree_recursive(&mut one_tree, one.as_mut_slice(), depth);

        *decision_tree = Some(Box::new(DecisionTreeNode::Branch {
            decision_bit,
            zero: zero_tree,
            one: one_tree,
            index: None,
        }));
        break;
    }

    *depth -= 1;
    log::debug!("Decision tree built at depth {}", depth);
}

fn assign_indexes_dfs(decision_tree: &mut DecisionTree, index: &mut usize) {
    if let Some(node) = decision_tree {
        match &mut **node {
            DecisionTreeNode::Leaf { index: i, .. } => {
                *i = Some(*index);
                *index += 1;
            }
            DecisionTreeNode::Branch {
                index: i,
                zero,
                one,
                ..
            } => {
                *i = Some(*index);
                *index += 1;
                assign_indexes_dfs(zero, index);
                assign_indexes_dfs(one, index);
            }
        }
    }
}

fn assign_indexes_bfs(decision_tree: &mut DecisionTree) {
    let mut index = 0;
    let mut queue = Vec::new();
    queue.push(decision_tree);

    while let Some(node) = queue.pop() {
        if let Some(node) = node {
            match &mut **node {
                DecisionTreeNode::Leaf { index: i, .. } => {
                    *i = Some(index);
                    index += 1;
                }
                DecisionTreeNode::Branch {
                    index: i,
                    zero,
                    one,
                    ..
                } => {
                    *i = Some(index);
                    index += 1;
                    queue.push(zero);
                    queue.push(one);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DecisionTreeIndexing {
    None,
    DFS,
    BFS,
}

fn assign_indexes(decision_tree: &mut DecisionTree, indexing: DecisionTreeIndexing) {
    match indexing {
        DecisionTreeIndexing::DFS => {
            let mut index = 0;
            assign_indexes_dfs(decision_tree, &mut index);
        }
        DecisionTreeIndexing::BFS => {
            assign_indexes_bfs(decision_tree);
        }
        DecisionTreeIndexing::None => {}
    }
}

pub fn build_decision_tree(insns: &[Rc<Insn>], indexing: DecisionTreeIndexing) -> DecisionTree {
    let mut decision_tree = None;
    let mut depth = 0;

    let insns = insns
        .iter()
        .map(|insn| LeafNode {
            insn: insn.clone(),
            mask: insn.mask,
        })
        .collect::<Vec<_>>();

    build_decision_tree_recursive(&mut decision_tree, insns.as_slice(), &mut depth);
    assign_indexes(&mut decision_tree, indexing);

    log::info!("Decision tree generated");
    log::trace!("Decision tree: {:x?}", decision_tree);

    decision_tree
}

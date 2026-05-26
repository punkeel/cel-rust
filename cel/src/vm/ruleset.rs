use crate::objects::Value;
use crate::vm::filter_tree::BoolFilter;

/// A compiled ruleset that partitions rules by the variables they access,
/// merges string-contains patterns per variable into Aho-Corasick automata,
/// and evaluates in dependency order.
///
/// This is the "option 2" architecture: instead of Vec<Box<dyn BoolFilter>>
/// with generic dispatch, we group rules by field and use specialized
/// evaluators for each group.
pub struct FieldPartitionedRuleset {
    // Rules that only touch a single int variable (fast path)
    pub int_rules: Vec<IntRule>,
    // Rules that only touch a single string variable (fast path)
    pub str_rules: Vec<StrRule>,
    // Rules with AC-merged contains patterns per variable
    pub ac_rules: Vec<ACRule>,
    // Rules that touch multiple variables or have complex structure
    pub complex_rules: Vec<Box<dyn BoolFilter>>,
    // Variable names in the order they appear in the vars slice
    pub var_names: Vec<String>,
}

/// An int-only rule: var op constant.
/// Inline, no heap, evaluable with a single match.
#[derive(Clone, Debug)]
pub enum IntRule {
    Eq { var_idx: usize, val: i64 },
    Ne { var_idx: usize, val: i64 },
    Lt { var_idx: usize, val: i64 },
    Le { var_idx: usize, val: i64 },
    Gt { var_idx: usize, val: i64 },
    Ge { var_idx: usize, val: i64 },
    InSet { var_idx: usize, vals: Vec<i64> },
}

impl IntRule {
    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> bool {
        let var_val = match vars.get(self.var_idx()) {
            Some(Value::Int(i)) => *i,
            _ => return false,
        };
        match self {
            Self::Eq { val, .. } => var_val == *val,
            Self::Ne { val, .. } => var_val != *val,
            Self::Lt { val, .. } => var_val < *val,
            Self::Le { val, .. } => var_val <= *val,
            Self::Gt { val, .. } => var_val > *val,
            Self::Ge { val, .. } => var_val >= *val,
            Self::InSet { vals, .. } => vals.contains(&var_val),
        }
    }

    fn var_idx(&self) -> usize {
        match self {
            Self::Eq { var_idx, .. }
            | Self::Ne { var_idx, .. }
            | Self::Lt { var_idx, .. }
            | Self::Le { var_idx, .. }
            | Self::Gt { var_idx, .. }
            | Self::Ge { var_idx, .. }
            | Self::InSet { var_idx, .. } => *var_idx,
        }
    }
}

/// A string-only rule: var op constant.
#[derive(Clone, Debug)]
pub enum StrRule {
    Eq { var_idx: usize, val: String },
    StartsWith { var_idx: usize, prefix: String },
    EndsWith { var_idx: usize, suffix: String },
}

impl StrRule {
    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> bool {
        let var_str = match vars.get(self.var_idx()) {
            Some(Value::String(s)) => s.as_str(),
            _ => return false,
        };
        match self {
            Self::Eq { val, .. } => var_str == val.as_str(),
            Self::StartsWith { prefix, .. } => var_str.starts_with(prefix),
            Self::EndsWith { suffix, .. } => var_str.ends_with(suffix),
        }
    }

    fn var_idx(&self) -> usize {
        match self {
            Self::Eq { var_idx, .. }
            | Self::StartsWith { var_idx, .. }
            | Self::EndsWith { var_idx, .. } => *var_idx,
        }
    }
}

/// Aho-Corasick rule: one scan per variable, multiple patterns.
pub struct ACRule {
    pub var_idx: usize,
    pub automaton: aho_corasick::AhoCorasick,
    pub min_matches: usize,
}

impl ACRule {
    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => {
                let mut matched = 0u64;
                for mat in self.automaton.find_iter(s.as_bytes()) {
                    let pid = mat.pattern().as_u64();
                    if pid < 64 {
                        matched |= 1u64 << pid;
                        if matched.count_ones() as usize >= self.min_matches {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}

impl FieldPartitionedRuleset {
    /// Evaluate all rules, return count of matches.
    #[inline(always)]
    pub fn eval_count(&self, vars: &[Value]) -> usize {
        let mut count = 0usize;
        for rule in &self.int_rules {
            if rule.eval(vars) {
                count += 1;
            }
        }
        for rule in &self.str_rules {
            if rule.eval(vars) {
                count += 1;
            }
        }
        for rule in &self.ac_rules {
            if rule.eval(vars) {
                count += 1;
            }
        }
        for rule in &self.complex_rules {
            if rule.eval(vars) {
                count += 1;
            }
        }
        count
    }

    /// Evaluate all rules into a pre-allocated bool slice.
    #[inline(always)]
    pub fn eval_into(&self, vars: &[Value], out: &mut [bool]) {
        let mut idx = 0usize;
        for rule in &self.int_rules {
            out[idx] = rule.eval(vars);
            idx += 1;
        }
        for rule in &self.str_rules {
            out[idx] = rule.eval(vars);
            idx += 1;
        }
        for rule in &self.ac_rules {
            out[idx] = rule.eval(vars);
            idx += 1;
        }
        for rule in &self.complex_rules {
            out[idx] = rule.eval(vars);
            idx += 1;
        }
    }
}

use crate::common::ast::operators;
use crate::common::ast::{ComprehensionExpr, EntryExpr, Expr, IdedExpr, LiteralValue};
use crate::objects::Value;
use crate::vm::bytecode::{Instr, Program, IDX_ACCU, IDX_ITER_ELEM};
use crate::Expression;
use std::collections::{HashMap, HashSet};

pub fn compile(expr: &Expression) -> Result<Program, String> {
    let mut c = Compiler::new();
    c.compile_expr(&expr.expr)?;
    c.instructions.push(Instr::Return);
    Ok(Program {
        constants: c.constants,
        var_names: c.var_names.into_iter().collect(),
        instructions: c.instructions,
    })
}

struct Compiler {
    constants: Vec<Value>,
    const_map: HashMap<String, u16>,
    var_names: Vec<String>,
    var_map: HashMap<String, u16>,
    instructions: Vec<Instr>,
    comp_scopes: Vec<HashMap<String, CompBinding>>,
}

#[derive(Clone, Copy)]
enum CompBinding {
    Iter,
    Accu,
}

impl Compiler {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            const_map: HashMap::new(),
            var_names: Vec::new(),
            var_map: HashMap::new(),
            instructions: Vec::new(),
            comp_scopes: Vec::new(),
        }
    }

    fn add_const(&mut self, v: Value) -> u16 {
        let key = format!("{:?}", v);
        if let Some(&idx) = self.const_map.get(&key) {
            return idx;
        }
        let idx = self.constants.len() as u16;
        self.constants.push(v.clone());
        self.const_map.insert(key, idx);
        idx
    }

    fn add_string_const(&mut self, s: &str) -> u16 {
        self.add_const(Value::String(std::sync::Arc::new(s.to_string())))
    }

    fn ensure_var(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.var_map.get(name) {
            return idx;
        }
        let idx = self.var_names.len() as u16;
        self.var_names.push(name.to_string());
        self.var_map.insert(name.to_string(), idx);
        idx
    }

    fn emit(&mut self, instr: Instr) -> usize {
        self.instructions.push(instr);
        self.instructions.len() - 1
    }

    fn emit_jump(&mut self, op: fn(i16) -> Instr) -> usize {
        let idx = self.emit(op(0));
        idx
    }

    fn patch_jump(&mut self, idx: usize, target: usize) {
        let offset = (target as isize) - ((idx + 1) as isize);
        let offset = offset as i16;
        self.instructions[idx] = match self.instructions[idx] {
            Instr::Jump(_) => Instr::Jump(offset),
            Instr::JumpIfFalse(_) => Instr::JumpIfFalse(offset),
            Instr::JumpIfTrue(_) => Instr::JumpIfTrue(offset),
            Instr::JumpIfFalseKeep(_) => Instr::JumpIfFalseKeep(offset),
            Instr::JumpIfTrueKeep(_) => Instr::JumpIfTrueKeep(offset),
            Instr::IterNext(_) => Instr::IterNext(offset),
            _ => panic!("patch_jump on non-jump"),
        };
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Literal(lit) => {
                let v = lit_to_value(lit);
                let idx = self.add_const(v);
                self.emit(Instr::PushConst(idx));
                Ok(())
            }
            Expr::Ident(name) => {
                if let Some(binding) = self.lookup_comp_var(name) {
                    match binding {
                        CompBinding::Iter => self.emit(Instr::LoadVar(IDX_ITER_ELEM)),
                        CompBinding::Accu => self.emit(Instr::LoadVar(IDX_ACCU)),
                    };
                } else {
                    let idx = self.ensure_var(name);
                    self.emit(Instr::LoadVar(idx));
                }
                Ok(())
            }
            Expr::Call(call) => self.compile_call(call),
            Expr::Select(sel) => self.compile_select(sel),
            Expr::Comprehension(comp) => self.compile_comprehension(comp),
            Expr::List(list) => {
                for elem in &list.elements {
                    self.compile_expr(&elem.expr)?;
                }
                self.emit(Instr::BuildList(list.elements.len() as u16));
                Ok(())
            }
            Expr::Map(map) => {
                for entry in &map.entries {
                    match &entry.expr {
                        EntryExpr::MapEntry(e) => {
                            self.compile_expr(&e.key.expr)?;
                            self.compile_expr(&e.value.expr)?;
                        }
                        _ => return Err("unsupported map entry".into()),
                    }
                }
                self.emit(Instr::BuildMap(map.entries.len() as u16));
                Ok(())
            }
            _ => Err(format!(
                "unsupported expr: {:?}",
                std::mem::discriminant(expr)
            )),
        }
    }

    fn compile_call(&mut self, call: &crate::common::ast::CallExpr) -> Result<(), String> {
        let name = call.func_name.as_str();

        // Binary ops
        if call.args.len() == 2 {
            match name {
                operators::ADD => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Add);
                    return Ok(());
                }
                operators::SUBSTRACT => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Sub);
                    return Ok(());
                }
                operators::MULTIPLY => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Mul);
                    return Ok(());
                }
                operators::DIVIDE => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Div);
                    return Ok(());
                }
                operators::MODULO => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Mod);
                    return Ok(());
                }
                operators::EQUALS => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Eq);
                    return Ok(());
                }
                operators::NOT_EQUALS => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Ne);
                    return Ok(());
                }
                operators::LESS => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Lt);
                    return Ok(());
                }
                operators::LESS_EQUALS => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Le);
                    return Ok(());
                }
                operators::GREATER => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Gt);
                    return Ok(());
                }
                operators::GREATER_EQUALS => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Ge);
                    return Ok(());
                }
                operators::LOGICAL_AND => {
                    self.compile_expr(&call.args[0].expr)?;
                    let jump = self.emit_jump(Instr::JumpIfFalseKeep);
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::LogicalAnd);
                    self.patch_jump(jump, self.instructions.len());
                    return Ok(());
                }
                operators::LOGICAL_OR => {
                    self.compile_expr(&call.args[0].expr)?;
                    let jump = self.emit_jump(Instr::JumpIfTrueKeep);
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::LogicalOr);
                    self.patch_jump(jump, self.instructions.len());
                    return Ok(());
                }
                operators::IN => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::In);
                    return Ok(());
                }
                operators::INDEX => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Index);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Unary ops
        if call.args.len() == 1 {
            match name {
                operators::LOGICAL_NOT => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.emit(Instr::Not);
                    return Ok(());
                }
                operators::NEGATE => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.emit(Instr::Neg);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Ternary / conditional
        if call.args.len() == 3 && name == operators::CONDITIONAL {
            self.compile_expr(&call.args[0].expr)?;
            let jump_false = self.emit_jump(Instr::JumpIfFalse);
            self.compile_expr(&call.args[1].expr)?;
            let jump_end = self.emit_jump(Instr::Jump);
            self.patch_jump(jump_false, self.instructions.len());
            self.compile_expr(&call.args[2].expr)?;
            self.patch_jump(jump_end, self.instructions.len());
            return Ok(());
        }

        // Function calls
        for arg in &call.args {
            self.compile_expr(&arg.expr)?;
        }
        let builtin_id = match name {
            "size" => 0,
            "int" => 1,
            "double" => 2,
            "uint" => 3,
            "string" => 4,
            "max" => 5,
            _ => return Err(format!("unsupported function: {}", name)),
        };
        let argc = call.args.len() as u16;
        self.emit(Instr::Call(builtin_id, argc));
        Ok(())
    }

    fn compile_select(&mut self, sel: &crate::common::ast::SelectExpr) -> Result<(), String> {
        self.compile_expr(&sel.operand.expr)?;
        let field_idx = self.add_string_const(&sel.field);
        if sel.test {
            self.emit(Instr::HasField(field_idx));
        } else {
            self.emit(Instr::Select(field_idx));
        }
        Ok(())
    }

    fn compile_comprehension(
        &mut self,
        comp: &ComprehensionExpr,
    ) -> Result<(), String> {
        self.compile_expr(&comp.iter_range.expr)?;
        self.emit(Instr::IterInit);

        // Accumulator
        if let Expr::Literal(lit) = &comp.accu_init.expr {
            let v = lit_to_value(lit);
            let idx = self.add_const(v);
            self.emit(Instr::AccuPush(idx));
        } else {
            self.compile_expr(&comp.accu_init.expr)?;
            self.emit(Instr::AccuPush(0xFFFF));
        }

        // Loop
        let loop_start = self.instructions.len();
        let iter_next_idx = self.emit_jump(Instr::IterNext);

        let mut scope = HashMap::new();
        scope.insert(comp.iter_var.clone(), CompBinding::Iter);
        scope.insert(comp.accu_var.clone(), CompBinding::Accu);
        self.comp_scopes.push(scope);

        // Condition
        self.compile_expr(&comp.loop_cond.expr)?;
        let cond_jump = self.emit_jump(Instr::JumpIfFalseKeep);
        self.emit(Instr::Pop);

        // Loop step
        self.compile_expr(&comp.loop_step.expr)?;
        self.emit(Instr::AccuSet);
        self.emit(Instr::Pop);

        self.emit(Instr::Jump(0));
        let jump_back_idx = self.instructions.len() - 1;
        self.patch_jump(jump_back_idx, loop_start as usize);

        // Exit early
        self.patch_jump(cond_jump, self.instructions.len());
        self.emit(Instr::Pop);
        self.emit(Instr::Pop);

        // Done (iterator exhausted)
        self.patch_jump(iter_next_idx, self.instructions.len());
        self.emit(Instr::IterPop);

        // Result
        self.compile_expr(&comp.result.expr)?;

        self.comp_scopes.pop();

        Ok(())
    }

    fn lookup_comp_var(&self, name: &str) -> Option<CompBinding> {
        for scope in self.comp_scopes.iter().rev() {
            if let Some(&binding) = scope.get(name) {
                return Some(binding);
            }
        }
        None
    }
}

fn lit_to_value(lit: &LiteralValue) -> Value {
    match lit {
        LiteralValue::Boolean(b) => Value::Bool(*b.inner()),
        LiteralValue::Int(i) => Value::Int(*i.inner()),
        LiteralValue::UInt(u) => Value::UInt(*u.inner()),
        LiteralValue::Double(f) => Value::Float(*f.inner()),
        LiteralValue::String(s) => Value::String(std::sync::Arc::new(s.inner().to_string())),
        LiteralValue::Bytes(b) => Value::Bytes(std::sync::Arc::new(b.inner().to_vec())),
        LiteralValue::Null => Value::Null,
    }
}

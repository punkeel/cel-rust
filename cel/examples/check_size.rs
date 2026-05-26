use cel::vm::bytecode::Instr;

fn main() {
    println!("sizeof Instr = {}", std::mem::size_of::<Instr>());
}

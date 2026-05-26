use cel::Program;

fn main() {
    for expr in &["port == 80", "port >= 1024", "port in [80, 8080, 8880, 2052, 2082, 2086, 2095]"] {
        let prog = Program::compile(expr).unwrap();
        let vm = prog.compile_vm().unwrap();
        println!("=== {} ===", expr);
        for (i, instr) in vm.instructions.iter().enumerate() {
            println!("  {:3}: {:?}", i, instr);
        }
    }
}

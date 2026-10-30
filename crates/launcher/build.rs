fn main() {
    protobuf_codegen::Codegen::new()
        .pure()
        .include("proto")
        .input("proto/elephant.proto")
        .cargo_out_dir("proto")
        .run_from_script();
}

use std::env;
use std::path::PathBuf;
use w4dj::w4dj_library::{W4djLibrary, write_emotion_evaluation_manifest};

fn option_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    let database = option_value(&args, "--database")
        .or_else(|| env::var("W4DJ_LIBRARY_PATH").ok())
        .ok_or("缺少 --database <w4dj.sqlite3> 或 W4DJ_LIBRARY_PATH")?;
    let output = option_value(&args, "--output").ok_or("缺少 --output <manifest.json>")?;
    let count = option_value(&args, "--count")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(100);
    let seed = option_value(&args, "--seed")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(1);

    let library = W4djLibrary::open(PathBuf::from(database).as_path())?;
    let manifest = library.emotion_evaluation_manifest(count, seed)?;
    write_emotion_evaluation_manifest(PathBuf::from(output).as_path(), &manifest)?;
    println!("已导出 {} 首歌曲的情绪验收 manifest", manifest.sample_size);
    Ok(())
}

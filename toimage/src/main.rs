//! A CLI tool for converting a DICOM image file
//! into a general purpose image file (e.g. PNG).
use std::path::PathBuf;

use dicom::object::open_file;
use dicom_pixeldata::PixelDecoder;
use snafu::ErrorCompat;
use structopt::StructOpt;

/// Convert a DICOM file into an image
#[derive(Debug, StructOpt)]
struct App {
    /// Path to the DICOM file to read
    file: PathBuf,
    /// Path to the output image
    /// (default is to replace input extension with `.png`)
    #[structopt(short = "o", long = "out")]
    output: Option<PathBuf>,
    /// Frame number (0-indexed)
    #[structopt(short = "F", long = "frame", default_value = "0")]
    frame_number: u16,
    /// Print more information about the image and the output file
    #[structopt(short="v", long="verbose")]
    verbose: bool
}


fn report<E: 'static>(err: &E)
where
    E: std::error::Error,
{
    eprintln!("[ERROR] {}", err);
    if let Some(source) = err.source() {
        eprintln!();
        eprintln!("Caused by:");
        for (i, e) in std::iter::successors(Some(source), |e| e.source()).enumerate() {
            eprintln!("   {}: {}", i, e);
        }
    }
}

fn report_backtrace<E: 'static>(err: &E)
where
    E: std::error::Error,
    E: ErrorCompat,
{
    let env_backtrace = std::env::var("RUST_BACKTRACE").unwrap_or_default();
    let env_lib_backtrace = std::env::var("RUST_LIB_BACKTRACE").unwrap_or_default();
    if env_lib_backtrace == "1" || (env_backtrace == "1" && env_lib_backtrace != "0") {
        if let Some(backtrace) = ErrorCompat::backtrace(&err) {
            eprintln!();
            eprintln!("Backtrace:");
            eprintln!("{}", backtrace);
        }
    }
}

fn report_with_backtrace<E: 'static>(err: E)
where
    E: std::error::Error,
    E: ErrorCompat,
{
    report(&err);
    report_backtrace(&err);
}


fn main() {
    let App {
        file,
        output,
        frame_number,
        verbose,
    } = App::from_args();

    let output = output.unwrap_or_else(|| {
        let mut path = file.clone();
        path.set_extension("png");
        path
    });

    let obj = open_file(&file).unwrap_or_else(|e| {
        report_with_backtrace(e);
        std::process::exit(-1);
    });

    let pixel = obj.decode_pixel_data().unwrap_or_else(|e| {
        report_with_backtrace(e);
        std::process::exit(-2);
    });

    if verbose {
        println!("{}x{}x{} image, {}-bit", pixel.cols, pixel.rows, pixel.samples_per_pixel, pixel.bits_stored);
    }

    let image = pixel.to_dynamic_image(frame_number).unwrap_or_else(|e| {
        report_with_backtrace(e);
        std::process::exit(-3);
    });

    let image = image.brighten(i32::from(pixel.rescale_intercept)).adjust_contrast(pixel.rescale_slope);

    image.save(&output).unwrap_or_else(|e| {
        report(&e);
        std::process::exit(-4);
    });

    if verbose {
        println!("Image saved to {}", output.display());
    }
}

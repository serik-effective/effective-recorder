use ffmpeg_next as ffmpeg;

fn main() {
    ffmpeg::init().unwrap();

    println!("=== Testing minimal video encode ===");
    match test_encode() {
        Ok(()) => println!("Encode test PASSED"),
        Err(e) => println!("Encode test FAILED: {}", e),
    }
}

fn test_encode() -> Result<(), Box<dyn std::error::Error>> {
    let output_path = "/tmp/test_screen_rec.mp4";
    let mut output = ffmpeg::format::output(output_path)?;

    let codec = ffmpeg::encoder::find_by_name("libx264")
        .ok_or("No H264 encoder found")?;
    println!("Using codec: {}", codec.name());

    let global_header = output.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);

    let mut stream = output.add_stream(codec)?;
    let stream_idx = stream.index();

    // Key fix: create context with codec, NOT from empty stream parameters
    let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
    let mut encoder = ctx.encoder().video()?;

    encoder.set_width(640);
    encoder.set_height(480);
    encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    encoder.set_time_base(ffmpeg::Rational(1, 30));
    encoder.set_frame_rate(Some(ffmpeg::Rational(30, 1)));
    encoder.set_gop(12);
    encoder.set_max_b_frames(2);

    if global_header {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::codec::flag::Flags::GLOBAL_HEADER.bits() as i32;
        }
    }

    let mut opts = ffmpeg::Dictionary::new();
    opts.set("preset", "ultrafast");
    opts.set("crf", "23");

    let mut encoder = encoder.open_as_with(codec, opts)?;
    stream.set_parameters(&encoder);

    output.write_header()?;
    println!("Header written OK");

    let time_base = output.stream(stream_idx).unwrap().time_base();

    for i in 0..30i64 {
        let mut frame =
            ffmpeg::util::frame::video::Video::new(ffmpeg::format::Pixel::YUV420P, 640, 480);
        for b in frame.data_mut(0).iter_mut() { *b = 16; }
        for b in frame.data_mut(1).iter_mut() { *b = 128; }
        for b in frame.data_mut(2).iter_mut() { *b = 128; }
        frame.set_pts(Some(i));

        encoder.send_frame(&frame)?;

        let mut packet = ffmpeg::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(stream_idx);
            packet.rescale_ts(ffmpeg::Rational(1, 30), time_base);
            packet.write_interleaved(&mut output)?;
        }
    }

    encoder.send_eof()?;
    let mut packet = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(stream_idx);
        packet.rescale_ts(ffmpeg::Rational(1, 30), time_base);
        packet.write_interleaved(&mut output)?;
    }

    output.write_trailer()?;

    let size = std::fs::metadata(output_path)?.len();
    println!("Output file size: {} bytes", size);

    Ok(())
}

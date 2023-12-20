use std::{fs, path::Path};

use clap::{Arg, Command};

fn get_zip_url(major: u32, minor: u32) -> reqwest::Url {
	reqwest::Url::parse(&format!(
		"https://secure.byond.com/download/build/{0}/{0}.{1}_byond.zip",
		major, minor
	))
	.unwrap()
}

fn main() {
	let matches = Command::new("byond_get")
		.disable_version_flag(true)
		.arg(
			Arg::new("major")
				.help("major BYOND version to fetch (e.g. 513)")
				.required(true),
		)
		.arg(
			Arg::new("minor")
				.help("minor BYOND version to fetch (e.g. 1539)")
				.required(true),
		)
		.arg(
			Arg::new("destination")
				.help("directory to extract the BYOND build into")
				.required(true),
		)
		.get_matches();

	let major = matches
		.get_one::<u32>("major")
		.expect("Major BYOND version must be an unsigned integer!");
	let minor = matches
		.get_one::<u32>("minor")
		.expect("Minor BYOND version must be an unsigned integer!");
	let destination = matches.get_one::<String>("destination").unwrap();
	let destination = Path::new(destination);

	if destination.exists() {
		panic!("path {:?} already exists", destination);
	}

	let resp = reqwest::blocking::get(get_zip_url(*major, *minor))
		.unwrap()
		.bytes()
		.unwrap();
	let stream = std::io::Cursor::new(resp);
	let mut archive = zip::ZipArchive::new(stream).unwrap();

	for i in 0..archive.len() {
		let mut file = archive.by_index(i).unwrap();

		if file.is_dir() {
			continue;
		}

		let local_path = file
			.enclosed_name()
			.unwrap()
			.strip_prefix("byond/")
			.unwrap();

		let mut path = destination.to_path_buf();
		path.push(local_path);

		std::fs::create_dir_all(path.parent().unwrap()).unwrap();

		println!("Extracting {} bytes to {:?}", file.size(), path);
		let mut out = fs::File::create(&path).unwrap();
		std::io::copy(&mut file, &mut out).unwrap();

		// TODO: posix perms
	}
}

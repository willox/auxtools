use std::{fs, path::Path};

use clap::{Arg, Command};

fn get_zip_url(major: u32, minor: u32) -> reqwest::Url {
	reqwest::Url::parse(&format!("https://secure.byond.com/download/build/{0}/{0}.{1}_byond.zip", major, minor)).unwrap()
}

fn main() {
	let matches = Command::new("byond_get")
		.disable_version_flag(true)
		.arg(
			Arg::new("major")
				.help("major BYOND version to fetch (e.g. 513)")
				.required(true)
				.takes_value(true)
		)
		.arg(
			Arg::new("minor")
				.help("minor BYOND version to fetch (e.g. 1539)")
				.required(true)
				.takes_value(true)
		)
		.arg(
			Arg::new("destination")
				.allow_invalid_utf8(true)
				.help("directory to extract the BYOND build into")
				.required(true)
				.takes_value(true)
		)
		.get_matches();

	let major = matches.value_of("major").unwrap();
	let minor = matches.value_of("minor").unwrap();
	let destination = matches.value_of_os("destination").unwrap();

	let major = major.parse::<u32>().expect("major version must be an integer");
	let minor = minor.parse::<u32>().expect("minor version must be an integer");
	let destination = Path::new(destination);

	if destination.exists() {
		panic!("path {:?} already exists", destination);
	}

	let resp = reqwest::blocking::get(get_zip_url(major, minor)).unwrap().bytes().unwrap();
	let stream = std::io::Cursor::new(resp);
	let mut archive = zip::ZipArchive::new(stream).unwrap();

	for i in 0..archive.len() {
		let mut file = archive.by_index(i).unwrap();

		if file.is_dir() {
			continue;
		}

		let local_path = file.enclosed_name().unwrap().strip_prefix("byond/").unwrap().to_owned();

		let mut path = destination.to_path_buf();
		path.push(local_path);

		std::fs::create_dir_all(path.parent().unwrap()).unwrap();

		println!("Extracting {} bytes to {:?}", file.size(), path);
		let mut out = fs::File::create(&path).unwrap();
		std::io::copy(&mut file, &mut out).unwrap();

		// TODO: posix perms
	}
}

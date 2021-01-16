use clap::{App, AppSettings, Arg};

fn get_zip_url(major: u32, minor: u32) -> reqwest::Url {
	reqwest::Url::parse(&format!(
		"https://secure.byond.com/download/build/{0}/{0}.{1}_byond.zip",
		major, minor
	))
	.unwrap()
}

fn main() {
	let matches = App::new("byond_get")
		.global_setting(AppSettings::DisableVersion)
		.arg(
			Arg::with_name("major")
				.help("major BYOND version to fetch (e.g. 513)")
				.required(true)
				.takes_value(true),
		)
		.arg(
			Arg::with_name("minor")
				.help("minor BYOND version to fetch (e.g. 1539)")
				.required(true)
				.takes_value(true),
		)
		.arg(
			Arg::with_name("destination")
				.help("directory to extract the BYOND build into")
				.required(true)
				.takes_value(true),
		)
		.get_matches();

	let version = matches.value_of("version");
	let destination = matches.value_of("destionation");

	println!("{:?} {:?}", version, destination);
	return;

	let resp = reqwest::blocking::get(get_zip_url(513, 1491))
		.unwrap()
		.bytes()
		.unwrap();
	let stream = std::io::Cursor::new(resp);
	let mut archive = zip::ZipArchive::new(stream).unwrap();

	for i in 0..archive.len() {
		let file = archive.by_index(i).unwrap();
		println!("{:?}", file.enclosed_name());
	}

	//println!("{:?}", resp);
}

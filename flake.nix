{
  inputs = {
	nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    fenix.url = "github:nix-community/fenix/720b54260dee864d2a21745bd2bb55223f58e297";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
	nixpkgs,
    fenix,
	naersk,
    ...
  }:
	let
  		toolchain = fenix.packages.i686-linux.stable.toolchain;
		pkgs = nixpkgs.legacyPackages.x86_64-linux.pkgsi686Linux;
	in
	{
		packages.x86_64-linux.default = (naersk.lib.x86_64-linux.override {
			cargo = toolchain;
			rustc = toolchain;
		}).buildPackage {
			src = ./.;
			copyLibs = true;
			nativeBuildInputs = [ pkgs.stdenv.cc pkgs.openssl_1_1 pkgs.pkg-config ];
			CARGO_BUILD_TARGET = "i686-unknown-linux-gnu";
          	CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER = "cc";
		};
	};
}

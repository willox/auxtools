#!/bin/env python3

# This script will return JSON representing all the unmatched signatures, and for what reason.

# The only library this script needs is `pyelftools`.

from sys import argv, stderr, exit
import re
from os import SEEK_CUR

from elftools.elf.elffile import ELFFile

def print_stderr(text):
    stderr.write(text + "\n")


UNIX_GET_BUILD_NUMBER_SYMBOL = "_ZN8ByondLib13GetByondBuildEv"

def determine_platform(file):
    file.seek(0)
    magic_bytes = file.read(4)

    if magic_bytes.startswith(b"MZ"):
        platform = "windows"
    elif magic_bytes.startswith(b"\x7fELF"):
        platform = "unix"
    else:
        print_stderr("Unknown BYOND binary provided.")
        exit(1)

    return platform

def determine_version(file):
    platform = determine_platform(file)

    file.seek(0)

    # since the unix binaries apparently don't expose the version number in any metadata,
    # we have to extract the build number off an exported function.
    if platform == "unix":
        elffile = ELFFile(file)
        dynsym = elffile.get_section_by_name(".dynsym")
        symbol_matches = dynsym.get_symbol_by_name(UNIX_GET_BUILD_NUMBER_SYMBOL)

        if symbol_matches is None:
            print_stderr(
                f"Failed to find '{UNIX_GET_BUILD_NUMBER_SYMBOL}'. Report this issue, with the BYOND version!"
            )
            exit(1)

        func_addr = symbol_matches[0].entry.st_value
        func_size = symbol_matches[0].entry.st_size
        file.seek(func_addr)

        func_data = file.read(func_size)
        build_number_offset = func_data.index(b"\xb8") + 1
        return int.from_bytes(
            func_data[build_number_offset : build_number_offset + 4], "little"
        )
    # windows is easier, there's .rsrc metadata exposing the build number.
    # although, it is an unconventional "5.0.<major version>.<build number> (<stuff>)" format,
    # so you can do a simple regex to grab the build number off it.
    elif platform == "windows":
        KEY_NAME = "ProductVersion".encode("utf-16-le")
        GET_BUILD_FROM_PRODUCTVERSION = re.compile(r"\d\.\d\.\d{3}\.(\d{4})")

        file_data = file.read()
        key_index = file_data.index(KEY_NAME)
        value_index = key_index + len(KEY_NAME) + 2
        value_end = file_data.index(b"\0\0", value_index) + 1
        value = file_data[value_index:value_end].decode("utf-16-le")

        return int(GET_BUILD_FROM_PRODUCTVERSION.match(value).group(1))

HEX_PAIR = "(?:[\\dabcdefABCDEF?]{2} ?)"

STRIP_OFFSETS = re.compile(f'(!?)\\((?:\\d|call), "({HEX_PAIR}+)"\\)')
STRIP_COMMENTS = re.compile(r"\/*.+?\*\?|//.*")
SIGNATURE_DECLARATION = re.compile(
    r"([\w_]+?) => (universal_signature|version_dependent_signature)!\((.+?)\)"
)
VERSION_DEPENDANT_RANGES = re.compile(f'([\\d.]+?) => "({HEX_PAIR}+)"')


def dump_signatures(data, prologue, version):
    ret = {}
    if prologue not in data:
        print_stderr(
            "signatures! macro call not found. Is the file badly written or is the script outdated?"
        )
        exit(1)

    signatures_start = data.index(prologue) + len(prologue)
    signatures_end = data.index("}", signatures_start)

    signatures = data[signatures_start:signatures_end]

    # the pattern also strips parenthesis, which is bad if the offset is directly within a macro call, so check for the exclamation mark and don't strip parenthesis if it has one.
    signatures = STRIP_OFFSETS.sub(
        lambda m: f'!("{m.group(2)}")' if m.group(1) else f'"{m.group(2)}"', signatures
    )
    signatures = STRIP_COMMENTS.sub("", signatures)
    signatures = signatures.replace("\n", "")

    for sig in SIGNATURE_DECLARATION.finditer(signatures):
        match sig.group(2):
            case "universal_signature":
                ret.update({sig.group(1): sig.group(3).replace('"', "")})
            case "version_dependent_signature":
                for version_range in VERSION_DEPENDANT_RANGES.finditer(sig.group(3)):
                    version_specifier = version_range.group(1)
                    if version_specifier.startswith(".."):
                       if version < int(version_specifier[2:]):
                          ret.update({sig.group(1): version_range.group(2)})
                          break
                    elif version_specifier.endswith(".."):
                        if version >= int(version_specifier[0:-2]):
                            ret.update({sig.group(1): version_range.group(2)})
                            break

                    # if ".." isn't in the version specifier, assume it's something like `1234 => "AB CD EF"`
                    if ".." not in version_specifier:
                       if int(version_specifier) == version:
                          ret.update({sig.group(1): version_range.group(2)})
                          break

                    # at last, assume it's something like `1234..5678 => "AB CD EF"`
                    version_start, version_end = version_specifier.split("..")
                    if version >= int(version_start) and version < int(version_end):
                       ret.update({sig.group(1): version_range.group(2)})
                       break
    return ret

if len(argv) < 3:
    print_stderr(f"Usage: {argv[0]} [path to auxtools repo] [path to byondcode/libbyond]")
    exit(1)

auxtools_main_lib_path = argv[1] + "/auxtools/src/lib.rs"
auxtools_instruction_hooking_path = argv[1] + "/instruction_hooking/src/lib.rs"

try:
    auxtools_main_lib = open(auxtools_main_lib_path, "r").read()
    auxtools_instruction_hooking = open(auxtools_instruction_hooking_path, "r").read()
    byondlib = open(argv[2], "rb", buffering=16 * 1024)
except Exception as e:
    print_stderr(f"Failure opening files: {e}\n")
    exit(1)


platform = determine_platform(byondlib)
version = determine_version(byondlib)

prelude = f"#[cfg({platform})]\nsignatures!"

sigs = dump_signatures(auxtools_main_lib, prelude, version)
sigs.update(dump_signatures(auxtools_instruction_hooking, prelude, version))

byondlib.seek(0)
lib_data = byondlib.read()

ret = {}

for i, signature_name in enumerate(sigs):
    signature_bytes = []

    for text_byte in sigs[signature_name].split(" "):
        if text_byte == "??":
            signature_bytes.append(0x1337)
        else:
            signature_bytes.append(int(text_byte, base=16))

    match_count = 0
    full_match = False

    i = 0
    for byte in lib_data:
        if byte == signature_bytes[match_count] or signature_bytes[match_count] == 0x1337:
            match_count += 1

            if match_count >= len(signature_bytes):
                if full_match:
                    ret.update({signature_name: "ERR(MULTIPLE MATCHES)"})
                    break

                full_match = True
                match_count = 0
        else:
            match_count = 0
        i += 1

    if not full_match:
        ret.update({signature_name: "ERR(NO MATCH)"})

print(ret)

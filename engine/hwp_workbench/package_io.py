"""Bound actual ZIP inflation, including members with forged size fields."""
import struct
import zipfile
import zlib

from .errors import require

# Bound each compressed read and each decompressor output allocation to 64 KiB.
ZIP_CHUNK = 64 * 1024


def read_zip_member(archive, info, expanded, *, part_limit, total_limit):
    """Read complete member data without ZipExtFile's declared-size truncation."""
    require(info.file_size <= part_limit and expanded + info.file_size <= total_limit,
            'LIMIT', 'Expanded ZIP exceeds limits.')
    # Keep zipfile's local-header/name and overlapping-member validation. No
    # read occurs here: ZipExtFile would truncate inflation to info.file_size.
    with archive.open(info):
        pass
    source = archive.fp
    source.seek(info.header_offset)
    header = source.read(30)
    name_size, extra_size = struct.unpack_from('<HH', header, 26)
    source.seek(name_size + extra_size, 1)
    decoder = zlib.decompressobj(-15) if info.compress_type == zipfile.ZIP_DEFLATED else None
    output = bytearray()
    checksum = 0

    def append(chunk):
        nonlocal checksum
        actual_size = len(output) + len(chunk)
        require(actual_size <= part_limit and expanded + actual_size <= total_limit,
                'LIMIT', 'Expanded ZIP exceeds limits.')
        checksum = zlib.crc32(chunk, checksum)
        output.extend(chunk)

    remaining = info.compress_size
    while remaining:
        compressed = source.read(min(ZIP_CHUNK, remaining))
        require(bool(compressed), 'BAD_ZIP', 'Truncated ZIP member.')
        remaining -= len(compressed)
        if decoder is None:
            append(compressed)
            continue
        pending = compressed
        while True:
            chunk = decoder.decompress(pending, ZIP_CHUNK)
            append(chunk)
            pending = decoder.unconsumed_tail
            require(not decoder.unused_data, 'BAD_ZIP', 'Data follows the ZIP deflate stream.')
            if not pending and len(chunk) < ZIP_CHUNK:
                break
    if decoder is not None:
        require(decoder.eof, 'BAD_ZIP', 'Incomplete ZIP deflate stream.')
    require(len(output) == info.file_size, 'BAD_ZIP', 'Inflated ZIP length differs from its header.')
    require(checksum == info.CRC, 'BAD_ZIP', 'ZIP member CRC mismatch.')
    return bytes(output)

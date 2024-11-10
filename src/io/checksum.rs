// pre-calculates all u32 crc combinations
static CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut c = 0;
    while c < 256 {
        let mut crc = (c as u32) << 24;
        let mut byte = 0;
        while byte < 8 {
            crc = (crc << 1) ^ (-(((crc >> 31) & 1) as i32) as u32 & 0x04c11db7);
            byte += 1;
        }
        table[c] = crc;
        c += 1;
    }
    table
};

pub fn crc32(seq: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in seq {
        crc = (crc << 8) ^ CRC_TABLE[((crc >> 24) ^ (b as u32)) as usize]
    }
    crc
}

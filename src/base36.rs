const BASE: u64 = 36;
const BITS_IN_BYTE: usize = 8;
const INPUT_LENGTH: usize = 6;
const OUTPUT_MAX_LENGTH: usize = 10;
const ENCODING_TABLE: [char; BASE as usize] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];

pub fn encode(input: [u8; INPUT_LENGTH]) -> String {
    let mut number: u64 = 0;
    // Convert bytes to a single number
    for (i, &byte) in input.iter().enumerate() {
        number |= (byte as u64) << (BITS_IN_BYTE * ((INPUT_LENGTH - 1) - i));
    }

    // Convert to base36
    let mut result = Vec::with_capacity(OUTPUT_MAX_LENGTH); // Max length needed for 6 bytes in base36

    while number > 0 {
        let remainder = (number % BASE) as usize;
        result.push(ENCODING_TABLE[remainder]);
        number /= BASE;
    }

    // Reverse and collect into string
    result.into_iter().rev().collect()
}

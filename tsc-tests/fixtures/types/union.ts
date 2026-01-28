type StringOrNumber = string | number;

const val1: StringOrNumber = "hello";
const val2: StringOrNumber = 42;

// TS2322
const val3: StringOrNumber = true;

function process(value: string | number): void {
    console.log(value);
}

process("test");
process(100);

// TS2345
process(false);

type MaybeString = string | null;

const maybe1: MaybeString = "exists";
const maybe2: MaybeString = null;

// TS2322
const maybe3: MaybeString = undefined;

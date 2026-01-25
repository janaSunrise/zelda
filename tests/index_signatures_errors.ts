// @errors: 2322, 2322, 2322

const dict: { [key: string]: number } = { a: "wrong" };

const dict2: { [key: string]: number } = { a: 1 };
const val: string = dict2.anyProp;

const obj: { name: string; [key: string]: string } = { name: 42 };

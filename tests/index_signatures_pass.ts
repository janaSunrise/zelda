const dict: { [key: string]: number } = { a: 1, b: 2 };
const val: number = dict.anyProperty;

const config: { name: string; [key: string]: string } = {
    name: "app",
    version: "1.0"
};

const key: string = "test";
const computed = dict[key];

interface StringMap {
    [key: string]: string;
}
const strDict: StringMap = { hello: "world", foo: "bar" };

interface Config {
    name: string;
    [key: string]: string;
}
const cfg: Config = { name: "myapp", extra: "value" };
const n: string = cfg.name;
const v: string = cfg.version;

const empty: { [key: string]: number } = {};

interface UserMap {
    [id: string]: { name: string; age: number };
}
const users: UserMap = {
    user1: { name: "Alice", age: 30 },
    user2: { name: "Bob", age: 25 }
};

interface Handlers {
    [event: string]: () => void;
}
const handlers: Handlers = {
    click: () => {},
    hover: () => {}
};

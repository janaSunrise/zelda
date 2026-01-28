class Counter {
    static count: number = 0;
    static prefix: string = "Counter";

    static increment(): number {
        return ++Counter.count;
    }

    static reset(): void {
        Counter.count = 0;
    }
}

const count1 = Counter.count;
const prefix = Counter.prefix;
Counter.increment();
Counter.reset();

// TS2322
Counter.count = "not a number";

// TS2339
const bad = Counter.nonexistent;

class Example {
    static staticMethod(): string {
        return "static";
    }

    instanceMethod(): string {
        return "instance";
    }
}

Example.staticMethod();
const ex = new Example();
ex.instanceMethod();

// TS2576
Example.instanceMethod();

// TS2339
ex.staticMethod();

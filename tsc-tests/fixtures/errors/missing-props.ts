interface Config {
    host: string;
    port: number;
    debug: boolean;
}

// TS2739 - missing properties
const config1: Config = {};
const config2: Config = { host: "localhost" };
const config3: Config = { host: "localhost", port: 3000 };

interface Database {
    connection: Config;
    name: string;
}

// TS2741 - nested missing
const db1: Database = { name: "mydb" };
const db2: Database = {
    connection: { host: "localhost" },
    name: "mydb"
};

function connect(config: Config): void {
    console.log(config);
}

// TS2345
connect({ host: "localhost" });

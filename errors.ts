const x = 1;
const x = 2;

// undefined references
const y = undefinedVar;

// block scoping
{
    const blockVar = 1;
}
const z = blockVar;

// shadowing
const a = 1;
function test() {
    const a = 2;
    return a;
}

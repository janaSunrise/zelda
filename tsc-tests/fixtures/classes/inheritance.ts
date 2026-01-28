class Animal {
    name: string;

    constructor(name: string) {
        this.name = name;
    }

    speak(): string {
        return `${this.name} makes a sound`;
    }
}

class Dog extends Animal {
    breed: string;

    constructor(name: string, breed: string) {
        super(name);
        this.breed = breed;
    }

    speak(): string {
        return `${this.name} barks`;
    }
}

const animal: Animal = new Dog("Buddy", "Labrador");

// TS2741
const dog1: Dog = new Animal("Generic");

const dogName = animal.name;

// TS2339
const dogBreed = animal.breed;

interface Flyable {
    fly(): void;
}

class Bird extends Animal implements Flyable {
    constructor(name: string) {
        super(name);
    }

    fly(): void {
        console.log(`${this.name} is flying`);
    }
}

const bird: Flyable = new Bird("Tweety");
bird.fly();

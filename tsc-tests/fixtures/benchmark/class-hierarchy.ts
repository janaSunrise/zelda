interface Serializable {
    serialize(): string;
}

interface Comparable<T> {
    compareTo(other: T): number;
}

interface Cloneable<T> {
    clone(): T;
}

abstract class Entity implements Serializable {
    readonly id: number;
    createdAt: Date;
    updatedAt: Date;

    constructor(id: number) {
        this.id = id;
        this.createdAt = new Date();
        this.updatedAt = new Date();
    }

    serialize(): string {
        return JSON.stringify(this);
    }

    abstract validate(): boolean;
}

class User extends Entity implements Comparable<User>, Cloneable<User> {
    username: string;
    email: string;
    passwordHash: string;
    isActive: boolean;

    constructor(id: number, username: string, email: string) {
        super(id);
        this.username = username;
        this.email = email;
        this.passwordHash = "";
        this.isActive = true;
    }

    validate(): boolean {
        return this.username.length > 0 && this.email.includes("@");
    }

    compareTo(other: User): number {
        return this.username.localeCompare(other.username);
    }

    clone(): User {
        const cloned = new User(this.id, this.username, this.email);
        cloned.passwordHash = this.passwordHash;
        cloned.isActive = this.isActive;
        return cloned;
    }
}

class Admin extends User {
    permissions: string[];
    level: number;

    constructor(id: number, username: string, email: string, level: number) {
        super(id, username, email);
        this.permissions = [];
        this.level = level;
    }

    addPermission(permission: string): void {
        this.permissions.push(permission);
    }

    hasPermission(permission: string): boolean {
        return this.permissions.includes(permission);
    }
}

class SuperAdmin extends Admin {
    canManageAdmins: boolean;

    constructor(id: number, username: string, email: string) {
        super(id, username, email, 100);
        this.canManageAdmins = true;
    }

    promoteToAdmin(user: User): Admin {
        return new Admin(user.id, user.username, user.email, 1);
    }
}

abstract class Repository<T extends Entity> {
    protected items: T[] = [];

    add(item: T): void {
        this.items.push(item);
    }

    findById(id: number): T | undefined {
        return this.items.find(item => item.id === id);
    }

    findAll(): T[] {
        return [...this.items];
    }

    remove(id: number): boolean {
        const index = this.items.findIndex(item => item.id === id);
        if (index >= 0) {
            this.items.splice(index, 1);
            return true;
        }
        return false;
    }

    abstract validate(item: T): boolean;
}

class UserRepository extends Repository<User> {
    findByUsername(username: string): User | undefined {
        return this.items.find(user => user.username === username);
    }

    findByEmail(email: string): User | undefined {
        return this.items.find(user => user.email === email);
    }

    validate(user: User): boolean {
        return user.validate() && !this.findByUsername(user.username);
    }
}

const user1 = new User(1, "john", "john@example.com");
const user2 = new User(2, "jane", "jane@example.com");
const admin = new Admin(3, "admin", "admin@example.com", 5);
const superAdmin = new SuperAdmin(4, "super", "super@example.com");

const repo = new UserRepository();
repo.add(user1);
repo.add(user2);
repo.add(admin);

const foundUser = repo.findById(1);
const foundByName = repo.findByUsername("john");
const allUsers = repo.findAll();
const comparison = user1.compareTo(user2);
const cloned = user1.clone();

// TS2345
const badUser = new User(1, 123, "email@test.com");
const badAdmin = new Admin("id", "admin", "email", 5);

// TS2322
const badAssign1: User = new Admin(1, "admin", "admin@test.com", 5);
const badAssign2: SuperAdmin = new Admin(1, "admin", "admin@test.com", 5);
const badAssign3: Admin = new User(1, "user", "user@test.com");

// TS2339
const badProp1 = user1.permissions;
const badProp2 = user1.level;
const badProp3 = admin.canManageAdmins;

// TS2554
const badArgs1 = new User(1, "john");
const badArgs2 = new Admin(1, "admin", "email");

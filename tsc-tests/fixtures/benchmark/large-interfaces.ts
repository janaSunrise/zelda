interface Address {
    street: string;
    city: string;
    state: string;
    zip: string;
    country: string;
}

interface ContactInfo {
    email: string;
    phone: string;
    fax: string | null;
    website: string | null;
}

interface Employment {
    company: string;
    position: string;
    startDate: string;
    endDate: string | null;
    salary: number;
    department: string;
}

interface Education {
    institution: string;
    degree: string;
    field: string;
    graduationYear: number;
    gpa: number | null;
}

interface Person {
    id: number;
    firstName: string;
    lastName: string;
    middleName: string | null;
    dateOfBirth: string;
    address: Address;
    contact: ContactInfo;
    employment: Employment[];
    education: Education[];
}

interface Company {
    id: number;
    name: string;
    legalName: string;
    taxId: string;
    founded: number;
    headquarters: Address;
    contact: ContactInfo;
    employees: Person[];
    revenue: number;
    industry: string;
}

interface Product {
    id: number;
    name: string;
    description: string;
    price: number;
    cost: number;
    sku: string;
    category: string;
    tags: string[];
    inStock: boolean;
    quantity: number;
}

interface Order {
    id: number;
    customer: Person;
    products: Product[];
    shippingAddress: Address;
    billingAddress: Address;
    total: number;
    tax: number;
    shipping: number;
    status: string;
    createdAt: string;
    updatedAt: string;
}

interface Invoice {
    id: number;
    order: Order;
    company: Company;
    dueDate: string;
    paidDate: string | null;
    amount: number;
    paid: boolean;
}

const validAddress: Address = {
    street: "123 Main St",
    city: "Springfield",
    state: "IL",
    zip: "62701",
    country: "USA"
};

const validContact: ContactInfo = {
    email: "test@example.com",
    phone: "555-1234",
    fax: null,
    website: "https://example.com"
};

const validPerson: Person = {
    id: 1,
    firstName: "John",
    lastName: "Doe",
    middleName: null,
    dateOfBirth: "1990-01-01",
    address: validAddress,
    contact: validContact,
    employment: [],
    education: []
};

// TS2322
const badAddress1: Address = {
    street: 123,
    city: "Springfield",
    state: "IL",
    zip: "62701",
    country: "USA"
};

const badAddress2: Address = {
    street: "123 Main St",
    city: "Springfield",
    state: "IL",
    zip: 62701,
    country: "USA"
};

// TS2741
const badPerson1: Person = {
    id: 1,
    firstName: "John"
};

const badPerson2: Person = {
    id: 1,
    firstName: "John",
    lastName: "Doe",
    middleName: null,
    dateOfBirth: "1990-01-01",
    address: validAddress,
    contact: validContact,
    employment: []
};

// TS2339
const badAccess1 = validPerson.ssn;
const badAccess2 = validAddress.apartment;
const badAccess3 = validContact.mobile;

const company: Company = {
    id: 1,
    name: "Acme Corp",
    legalName: "Acme Corporation Inc.",
    taxId: "12-3456789",
    founded: 1990,
    headquarters: validAddress,
    contact: validContact,
    employees: [validPerson],
    revenue: 1000000,
    industry: "Technology"
};

const employeeCity = company.employees[0].address.city;
const badNestedAccess = company.employees[0].address.county;

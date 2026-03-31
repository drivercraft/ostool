import Ajv2020 from "ajv/dist/2020";

export const jsonFormsAjv = new Ajv2020({
  allErrors: true,
  strict: false,
});

jsonFormsAjv.addFormat("uint32", {
  type: "number",
  validate: (value: number) =>
    Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff,
});

jsonFormsAjv.addFormat("uint64", {
  type: "number",
  validate: (value: number) => Number.isInteger(value) && value >= 0,
});

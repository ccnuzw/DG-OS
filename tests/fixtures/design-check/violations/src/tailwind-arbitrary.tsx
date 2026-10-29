export function Classes() {
  return (
    <div>
      <p className="bg-[#fff] p-2" />
      <p className="p-[13px]" />
      <p className="hover:w-[--radix-select-trigger-width]" />
      <p className="min-[400px]:p-2" />
      <p className="[&>svg]:size-4" />
      <p className="[--x:1px]" />
      <p className={`text-sm ${'gap-[3px]'}`} />
      <svg fill="red" stroke="Blue" />
    </div>
  );
}

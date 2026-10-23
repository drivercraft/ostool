import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { admin, account } from "@/api/events";
import type {
  ListFilters,
  ListPage,
  ManagedUser,
  Rental,
  RentalRequest,
} from "@/types/api";
import { ListPagination } from "./list-pagination";

export const queryKey = (query: object) =>
  JSON.stringify(
    Object.entries(query)
      .filter(([, v]) => v !== undefined && v !== null && v !== "")
      .map(([k, v]) => [
        k,
        k === "from" || k === "to" ? Date.parse(String(v)) : v,
      ])
      .sort(([a], [b]) => String(a).localeCompare(String(b))),
  );

type Rows = {
  users: ManagedUser;
  rentals: Rental;
  rental_requests: RentalRequest;
};
/** Server pagination; input stays immediate, search subscriptions are debounced. */
export function useServerList<K extends keyof Rows>(
  view: K,
  search: Omit<ListFilters, "view" | "page" | "page_size">,
  scope: "admin" | "account" = "admin",
) {
  const store = scope === "account" ? account : admin;
  const [selection, setSelection] = useState({
    key: queryKey(search),
    page: 1,
    size: 20,
  });
  const searchKey = queryKey(search);
  const page = selection.key === searchKey ? selection.page : 1;
  if (selection.key !== searchKey)
    setSelection({ ...selection, key: searchKey, page: 1 });
  const filters: ListFilters = {
    ...search,
    view,
    page,
    page_size: selection.size,
  };
  const identity = queryKey(filters);
  const previousSearch = useRef<string | undefined>(undefined);
  useEffect(() => {
    const delayed =
      previousSearch.current !== undefined &&
      previousSearch.current !== search.q;
    previousSearch.current = search.q;
    if (!delayed) {
      store.setListingFilters(filters);
      return;
    }
    const timer = setTimeout(() => store.setListingFilters(filters), 300);
    return () => clearTimeout(timer);
  }, [identity]);
  useEffect(() => () => store.setListingFilters(null), []);
  const raw = useSyncExternalStore(store.subscribe, () => store.get("listing"));
  const data =
    raw?.query?.view === view ? (raw as ListPage<Rows[K]>) : undefined;
  const error = useSyncExternalStore(store.subscribe, store.getErrors).listing;
  const ready = !!data && queryKey(data.query) === identity;
  useEffect(() => {
    if (ready && data.page !== page)
      setSelection((old) => ({ ...old, page: data.page }));
  }, [ready, data?.page, page]);
  return {
    rows: data?.items ?? [],
    total: data?.total ?? 0,
    error,
    loading: !data && !error,
    updating: !ready && !!data && !error,
    viewKey: data ? queryKey(data.query) : identity,
    footer: (
      <ListPagination
        total={data?.total ?? 0}
        page={data?.page ?? page}
        size={selection.size}
        pending={!ready}
        onPage={(page) => setSelection({ ...selection, page })}
        onSize={(size) => setSelection({ ...selection, page: 1, size })}
      />
    ),
  };
}
